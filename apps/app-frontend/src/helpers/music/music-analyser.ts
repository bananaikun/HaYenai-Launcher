/* ============================================================
   音乐响度分析（「雨随音乐」的数据源）
   ------------------------------------------------------------
   模块级单例，故意不走 React context：往里喂数据的是 MusicProvider 里的
   <audio>，读数据的是 Backdrop 里水面的每帧循环。两者在组件树上的先后
   关系不该决定这个功能能不能用，所以用模块单例把这一层依赖切掉。

   ⚠ 三条硬约束（改这个文件前必读）
   1. createMediaElementSource 一接管，该元素的音频就**只能**从 Web Audio
      图里出去 —— 必须自己 connect 到 destination，否则音乐直接变哑。
      而且接管不可逆：元素一旦接上就再也回不到「直连扬声器」。
      所以只在确实要用时才接（waterMusic > 0）。
   2. 跨域且没有 CORS 头的资源，MediaElementSource 会**静默输出静音**：
      不报错、不抛异常，只是能量恒为 0。调用方因此必须给 audio 设
      crossOrigin='anonymous'，并在加载失败时退回直连（见 music.tsx）。
      实测本项目音源 api.qijieya.cn 与其 CDN 都返回 access-control-allow-origin: *。
   3. AudioContext 在用户手势之前是 suspended 的。本模块由 audio 的 onPlay
      触发，正好在手势里，所以在这里 resume 是安全的。
   ============================================================ */

const FFT = 512;
/* 只统计中低频（人耳能量集中区）。全带平均会被高频那一片近乎为零的
   bin 严重拉低 —— 测出来「安静的歌」和「吵闹的歌」差别全被抹平。 */
const BAND = 0.45;
/* 声压映射窗口：FLOOR_DB 记 0、CEIL_DB 记 1。
   ⚠ 频谱一律走 getFloatFrequencyData 拿**真实 dBFS**，不要用 getByteFrequencyData：
   后者在 maxDecibels（AnalyserNode 默认值是 **-30 dBFS**）处就钳到 255，而音乐的
   低频段基本都在这条线之上 ⇒ 频段值恒为 1，可视化全程顶格。
   这就是「壁纸音乐条始终处于最大值」的根因 —— 与放什么歌无关，是取样方式的问题。 */
const FLOOR_DB = -60;
const CEIL_DB = -6;
const SPAN_DB = CEIL_DB - FLOOR_DB;

/** 段 [a,b) 的平均线性幅度 → dBFS。float 数据里静音 bin 是 -Infinity，必须挡掉 */
function bandDb(fb: Float32Array<ArrayBuffer>, a: number, b: number): number {
  let sum = 0;
  for (let k = a; k < b; k++) {
    const v = fb[k];
    if (Number.isFinite(v)) sum += Math.pow(10, v / 20);
  }
  return 20 * Math.log10(sum / Math.max(1, b - a) + 1e-6);
}

/** dBFS → 0..1（窗口外钳制）。窗口见 FLOOR_DB / CEIL_DB */
function dbToRaw(db: number): number {
  const v = (db - FLOOR_DB) / SPAN_DB;
  return v < 0 ? 0 : v > 1 ? 1 : v;
}

let ac: AudioContext | null = null;
let srcNode: MediaElementAudioSourceNode | null = null;
let an: AnalyserNode | null = null;
/* 显式写成 Float32Array<ArrayBuffer>：TS 5.7+ 起 TypedArray 默认是
   `…<ArrayBufferLike>`，而 getFloatFrequencyData 只接受 ArrayBuffer 那个 ——
   于是 `new Float32Array(n)` 推出来的类型反而传不进去。 */
let bins: Float32Array<ArrayBuffer> | null = null;
let hooked: HTMLAudioElement | null = null;
let level = 0;
let err = '';

/** 接管失败时的原因。调用方据此在界面上如实说明，而不是让滑杆静默失灵 */
export const analyserError = () => err;

/* ---------------- 可用性广播 ----------------
   壁纸包（WeWallpaper）要按「分析图此刻给不给得出真实频谱」决定：
   挂我们的频谱源、还是让渲染器照旧用它内置的模拟源。它没法每帧来问
   （挂载是一次性的），所以这里做一次广播：订阅当下先报一次现状，
   之后接上 / 判死时再报。

   ⚠ 订阅者的异常**必须在这里吞掉**：本函数是在 hookAudio 的 try 里被调的，
   抛出去会被那句 catch（它是给「接管失败」准备的）误判成接管失败，
   于是整条音频链路被标记成不可用、`noCors` 兜底也跟着误触发。 */
const readySubs = new Set<(ok: boolean) => void>();

/**
 * 分析图此刻能不能给出**真实**频谱。
 *
 * ⚠ 判据必须含 `!hooked.paused`，不能只看 `ac.state`：暂停播放时 AudioContext
 * 仍是 'running'（它不知道元素停了），此时 getAllFrequencyData 拿到的是一整片
 * dBFS=-Infinity ⇒ 频谱恒 0。少了这一条，下面 spectrumSnapshot 就会持续回一份
 * 「全 0 但形状合法」的对象 —— 而壁纸库判的是**对象在不在**、不是响不响，
 * 于是它把内置模拟源顶掉、改用这串 0，音画驱动的壁纸整体静止。
 * （主人 2026-10-04 报的「WE 壁纸卡成静态」；见 WeWallpaper.attachAudio 的兜底轮询。）
 */
export const analyserLive = (): boolean =>
  !!(an && bins && ac?.state === 'running' && hooked && !hooked.paused);

function emitReady(ok: boolean): void {
  for (const cb of readySubs) {
    try {
      cb(ok);
    } catch {
      /* 订阅者自己的问题，与「接管」无关 */
    }
  }
}

/** 订阅可用性。返回退订函数；订阅时会**立刻**回调一次当前状态 */
export function onAnalyserReady(cb: (ok: boolean) => void): () => void {
  readySubs.add(cb);
  emitReady(analyserLive());
  return () => {
    readySubs.delete(cb);
  };
}

/**
 * 把一个 audio 元素接进分析图。
 * 可以重复调用：元素没变就直接返回；元素换了（不可播曲目会让 React
 * 卸载并重建 <audio>）就接新的那个 —— 旧元素的图已经随元素一起没了。
 */
export function hookAudio(el: HTMLAudioElement): void {
  if (el === hooked) return;
  try {
    if (!ac) {
      const Ctor = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
      ac = new Ctor();
    }
    /* 先连到扬声器，再接手：反过来的话中间那几毫秒是真静音 */
    srcNode = ac.createMediaElementSource(el);
    an = ac.createAnalyser();
    an.fftSize = FFT;
    /* 0.8 是折中：太小雨量抖成闪烁，太大鼓点全被抹平 */
    an.smoothingTimeConstant = 0.8;
    bins = new Float32Array(new ArrayBuffer(an.frequencyBinCount * 4));
    srcNode.connect(an);
    an.connect(ac.destination);
    void ac.resume();
    hooked = el;
    err = '';
    emitReady(true);
  } catch (e) {
    /* 失败就彻底退回「不联动」，绝不让它影响播放本身 */
    an = null;
    srcNode = null;
    bins = null;
    hooked = null;
    err = (e as Error)?.message || String(e);
    emitReady(false);
  }
}

/**
 * 读一次响度，返回 0~1。
 * 水面每帧调一次，所以这里**不能**分配对象、不能碰 React 状态。
 * 起音快、回落慢：雨量跟着歌走，但不会随每个鼓点抽搐。
 */
export function readLevel(): number {
  if (!an || !bins || ac?.state !== 'running') {
    level *= 0.9;
    return level;
  }
  an.getFloatFrequencyData(bins);
  const n = Math.max(3, (bins.length * BAND) | 0);
  /* 从 2 起跳，跳过直流与最低那两个 bin —— 它们几乎恒为满值，
     会把整条曲线抬到一个跟音乐无关的基线上。
     bandDb 内部已做「转 dBFS 再线性映射」：响度是乘性的，直接对幅度取平均
     听起来永远「变化很小」。 */
  const raw = dbToRaw(bandDb(bins, 2, n));
  level = raw > level ? level + (raw - level) * 0.5 : level + (raw - level) * 0.08;
  return level;
}

/** 四段分频的段数。歌词条那条振幅带取这四段的**平均**（低 → 高）当整体响度。 */
export const BAND_COUNT = 4;
/** 频带上下界，占 frequencyBinCount 的比例，段间**对数**均分。
    上界取 0.62 而不是 1：最上面那三分之一 bin 在正常音乐里近乎恒为 0，
    圈进来只会把高段的量程摊薄。下界跳过最底下两个 bin —— 理由同 readLevel。 */
const BAND_LO = 2 / 256;
const BAND_HI = 0.62;
/** 逐段的平滑值。与 readLevel 同一套「起音快、回落慢」 */
const bandLevels = new Float32Array(BAND_COUNT);

/**
 * 读一次四段响度，写进 out（长度 ≥ BAND_COUNT）。
 * 和 readLevel 一样会被每帧调用 ⇒ 不分配对象、不碰 React 状态。
 * 分析器不可用时各段一起衰减回 0 —— 宁可让振幅带落回静息高度，也不假装有数据。
 */
export function readBands(out: Float32Array): void {
  if (!an || !bins || ac?.state !== 'running') {
    for (let i = 0; i < BAND_COUNT; i++) {
      bandLevels[i] *= 0.9;
      out[i] = bandLevels[i];
    }
    return;
  }
  an.getFloatFrequencyData(bins);
  const n = bins.length;
  const lo = BAND_LO * n;
  const ratio = (BAND_HI * n) / lo;
  for (let i = 0; i < BAND_COUNT; i++) {
    const a = Math.round(lo * Math.pow(ratio, i / BAND_COUNT));
    const b = Math.max(a + 1, Math.min(n, Math.round(lo * Math.pow(ratio, (i + 1) / BAND_COUNT))));
    const raw = dbToRaw(bandDb(bins, a, b));
    const cur = bandLevels[i];
    bandLevels[i] = raw > cur ? cur + (raw - cur) * 0.5 : cur + (raw - cur) * 0.08;
    out[i] = bandLevels[i];
  }
}

/* ---------------- 壁纸频谱（渲染器的 AudioSource 契约） ----------------
   为什么另做一份 64 段、而不是把上面那 4 段放大：契约是写死的 ——
   `snapshot()` 要回 `{left, right}`、各 **64 段**、值域 **0..1**
   （不足补零、多余截断，见 webwallgl/types.d.ts 的 AudioSource）。
   4 段喂进去，壁纸的音频可视化只会剩最左边四根柱子有反应。

   分桶与平滑口径与 readBands 完全一致（段间对数均分、起音快回落慢）——
   低段细、高段粗，与听感对齐；换个算法只会让两条可视化对不上。 */

/** 契约里的段数（webwallgl 的 AudioSource 要 64 段） */
export const SPECTRUM_BANDS = 64;

/** 我们只有一路声源（AnalyserNode 给的是混合后的单路）⇒ 左右共用同一份 */
const spectrum = new Float32Array(SPECTRUM_BANDS);
/** ⚠ 复用同一个对象和同一份数组：渲染循环**每帧**拉一次，这里一个字节都不该分配 */
const spectrumSource = { left: spectrum, right: spectrum };

/** 拉一次频谱快照。库的渲染循环每帧调它 —— 同 readLevel，不分配、不碰 React 状态 */
export function spectrumSnapshot(): { left: Float32Array; right: Float32Array } {
  if (an && bins && ac?.state === 'running') {
    an.getFloatFrequencyData(bins);
    const n = bins.length;
    const lo = BAND_LO * n;
    const ratio = (BAND_HI * n) / lo;
    for (let i = 0; i < SPECTRUM_BANDS; i++) {
      const a = Math.round(lo * Math.pow(ratio, i / SPECTRUM_BANDS));
      const b = Math.max(a + 1, Math.min(n, Math.round(lo * Math.pow(ratio, (i + 1) / SPECTRUM_BANDS))));
      const raw = dbToRaw(bandDb(bins, a, b));
      const cur = spectrum[i];
      spectrum[i] = raw > cur ? cur + (raw - cur) * 0.5 : cur + (raw - cur) * 0.08;
    }
  } else {
    /* 拿不到频谱就衰减回 0，而不是维持上一帧的假值 —— 壁纸侧的音频门控
       （粒子的 audioGate 之类）因此会如实停在静息态，不会挂着一张伪造的响度 */
    for (let i = 0; i < SPECTRUM_BANDS; i++) spectrum[i] *= 0.9;
  }
  return spectrumSource;
}

/**
 * 读一次**当下**频谱的峰值（0..1）。给不出频谱时是 0。
 *
 * ⚠ 为什么必须有它（主人 2026-10-05 报「音频条不跳了」）：
 * 渲染器判的是「音频源对象在不在」，不是「响不响」——
 * webwallgl 的 hostAudio.pump() 只做 `if (!src || !src.left || !src.right) active = false`，
 * 而 hostAudio.active 一旦为真，它的渲染循环就**不再调 simAudio.update(t)**：
 * 库内置的模拟源被顶掉、改用我们给的这一串。而 spectrumSnapshot 永远回一个
 * 形状合法的对象 ⇒ 只要我们把桥交出去、哪怕它恒为 0，音频条就彻底静止
 * （此前壁纸从没接过我们，一直走模拟源能动，所以这是接上之后才暴露的）。
 *
 * 光靠 analyserLive() 挡不住这一层：它覆盖的是「AudioContext 没在跑 / 元素暂停」，
 * 而「元素在放、输出却是静音」（被静音、音量 0、正在缓冲、跨域静默）时它照样是
 * true，getFloatFrequencyData 拿到的却是一片 -Infinity。调用侧因此必须再问一次
 * **现在真有能量吗**。
 *
 * 口径刻意用 0..1 值而不是 dB：要防的正是「交出去之后就是 0」，所以判据必须与
 * 交给库的那份数据同源（这里直接复用 spectrumSnapshot，不另算一套）。
 * 阈值留给调用方 —— 与库自己的 noiseFloor（createSpectrumCalibrator 里 0.02）对齐。
 */
export function spectrumPeak(): number {
  const s = spectrumSnapshot();
  let peak = 0;
  for (let i = 0; i < s.left.length; i++) {
    const v = s.left[i];
    if (v > peak) peak = v;
  }
  return peak;
}
