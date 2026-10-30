/* ============================================================
   音乐音源解析

   为什么单独成文件：音源这块的失败模式很特殊，必须把「上游出错」和
   「歌单真的为空」区分开。旧实现把两者都当成空歌单，用户被告知
   「歌单为空」后只会去反复检查歌单 ID —— 排查方向被彻底带偏。

   上游（Meting 兼容实例）出错时返回的是对象而不是数组，例如：
     {"message":"请求次数已达上限，请明天再试"}
   这种响应既没有 url 也没有 name，旧逻辑按 `r.url || r.name` 过滤时
   会被静默丢弃，最终表现为「歌单为空」。这里改为显式识别并抛出原文。

   还负责：
   - 多源自动降级（首选失败就依次试候选，首个成功即用）
   - 版权受限曲目**标记**为不可播，而不是从列表里删掉
   ============================================================ */

export type RawSong = {
  /** 网易云歌曲 id。Meting 各实现都会带回，但没保证，所以按可选处理 */
  id?: number | string;
  name?: string;
  artist?: string;
  url?: string;
  pic?: string;
  lrc?: string;
};

/** 候选音源：按顺序尝试。实测可用的放最前。 */
export const METING_SOURCES = [
  'https://api.qijieya.cn/meting/',
  'https://api.injahow.cn/meting/',
  'https://api.i-meto.com/meting/api',
  'https://meting.qjqq.cn/',
] as const;

/** 只取主机名：界面里说明「实际用了哪个源」时比整条 URL 好读 */
export const hostOf = (api: string) => {
  try {
    return new URL(api).host;
  } catch {
    return api;
  }
};

/** 上游把错误包在 JSON 里时常见的字段 */
const ERR_KEYS = ['message', 'msg', 'error', 'errmsg', 'detail'] as const;

/** 从任意上游返回体里提取「这是错误提示」的原文；不是错误则返回 null */
export function upstreamError(data: unknown): string | null {
  if (!data || typeof data !== 'object' || Array.isArray(data)) return null;
  const o = data as Record<string, unknown>;
  // 有歌可以放就说明不是纯错误体
  if (typeof o.url === 'string' && o.url) return null;
  if (typeof o.name === 'string' && o.name) return null;
  for (const k of ERR_KEYS) {
    const v = o[k];
    if (typeof v === 'string' && v.trim()) return v.trim();
  }
  const code = o.code;
  if (typeof code === 'number' && code >= 400) return `上游返回错误码 ${code}`;
  return null;
}

/** 把错误原文翻译成人话（保留原文，只补一句该怎么做） */
function explain(raw: string): string {
  if (/上限|频繁|limit|quota|too many/i.test(raw)) {
    return `音源配额已用尽（${raw}）。已自动尝试其它音源。`;
  }
  return `音源返回异常：${raw}`;
}

/**
 * 拼上游请求地址。
 * spec 支持两种形态：
 *   - 纯数字        → 按歌单 ID 取整个歌单
 *   - "song:1,2,3" 或 "1,2,3" → 按歌曲 ID 列表逐个取
 */
export function buildUrls(api: string, spec: string): string[] {
  const s = spec.trim();
  if (!s) throw new Error('尚未配置歌单 ID（设置 → 音乐与歌词）');
  const base = api.endsWith('/') ? api : `${api}/`;
  const q = (p: string) => `${base}?${p}`;

  const asSongList = s.includes(',') || s.startsWith('song:');
  if (asSongList) {
    const ids = s.replace(/^song:/, '').split(',').map((x) => x.trim()).filter(Boolean);
    if (!ids.length) throw new Error('歌曲 ID 列表为空');
    return ids.map((id) => q(`server=netease&type=song&id=${encodeURIComponent(id)}`));
  }
  if (!/^\d+$/.test(s)) {
    throw new Error(`歌单 ID 应为纯数字，当前填的是「${s}」。若是多首歌请用逗号分隔歌曲 ID。`);
  }
  return [q(`server=netease&type=playlist&id=${encodeURIComponent(s)}`)];
}

async function jsonGet(url: string, signal?: AbortSignal): Promise<unknown> {
  const r = await fetch(url, { signal });
  if (!r.ok) throw new Error(`音源 HTTP ${r.status}`);
  const text = await r.text();
  try {
    return JSON.parse(text);
  } catch {
    throw new Error(`音源返回了非 JSON 内容（前 60 字：${text.slice(0, 60).replace(/\s+/g, ' ')}）`);
  }
}

/** 从单个音源取回曲目原始数据 */
export async function fetchFromSource(api: string, spec: string, signal?: AbortSignal): Promise<RawSong[]> {
  const urls = buildUrls(api, spec);
  const chunks = await Promise.all(
    urls.map(async (u) => {
      const data = await jsonGet(u, signal);
      const err = upstreamError(data);
      if (err) throw new Error(explain(err));
      const arr = Array.isArray(data) ? data : [data];
      return arr as RawSong[];
    }),
  );
  const all = chunks.flat();
  const usable = all.filter((r) => r && (r.url || r.name));
  if (!usable.length) {
    if (!all.length) throw new Error('音源返回了空歌单');
    throw new Error('音源返回的数据里没有任何曲目字段（url / name 都缺失）');
  }
  return usable;
}

export type Resolved = {
  /** 实际取到数据的音源（可能与设置里的不同，说明发生了降级） */
  api: string;
  songs: RawSong[];
  /** 降级过程中被跳过的失败记录，用于「为什么换了源」的说明 */
  skipped: { api: string; error: string }[];
};

/**
 * 按歌曲 id 批量取原始数据。
 * Meting 不支持一次查多首，所以逐个请求并并发；**单个失败不影响其余**
 * （用 allSettled 的语义：内部 try 吞掉，位置对齐返回 null）。
 */
async function fetchSongIds(api: string, ids: number[], signal?: AbortSignal): Promise<(RawSong | null)[]> {
  const urls = buildUrls(api, `song:${ids.join(',')}`);
  return Promise.all(
    urls.map(async (u) => {
      try {
        const data = await jsonGet(u, signal);
        if (upstreamError(data)) return null;
        const arr = Array.isArray(data) ? data : [data];
        const first = arr[0] as RawSong | undefined;
        return first && first.url ? first : null;
      } catch {
        return null;
      }
    }),
  );
}

export type IdResolved = {
  api: string;
  /** 与传入 ids 一一对应；取不到播放地址的为 null */
  songs: (RawSong | null)[];
};

/**
 * 批量解析指定歌曲 id 的播放地址。
 * 某源一首都没取到就换下一个源；全部失败才抛错。
 * 用于「从网易云歌单加入播放队列」——会员曲目解析不到属正常情况，不计为失败。
 */
export async function resolveSongIds(
  preferred: string,
  ids: number[],
  signal?: AbortSignal,
): Promise<IdResolved> {
  const clean = ids.map(Number).filter(Number.isFinite);
  if (!clean.length) throw new Error('歌曲 ID 列表为空');
  const order = [...new Set([preferred, ...METING_SOURCES].map((x) => (x || '').trim()).filter(Boolean))];

  let lastErr: Error = new Error('所有音源都无法解析这些歌曲');
  for (const api of order) {
    try {
      const songs = await fetchSongIds(api, clean, signal);
      if (songs.some((x) => x)) return { api, songs };
      lastErr = new Error(`${api} 没有返回任何可播放地址`);
    } catch (e) {
      if ((e as Error).name === 'AbortError') throw e;
      lastErr = e as Error;
    }
  }
  throw lastErr;
}

/**
 * 多源解析：先试首选源，失败就依次试候选。首个成功即返回。
 * 全部失败时抛出汇总错误，列出每个源的失败原因 —— 用户据此就能判断是
 * 配额问题、网络问题，还是歌单本身的问题。
 */
export async function resolveSongs(
  preferred: string,
  spec: string,
  signal?: AbortSignal,
): Promise<Resolved> {
  const order = [...new Set([preferred, ...METING_SOURCES].map((x) => (x || '').trim()).filter(Boolean))];
  const skipped: { api: string; error: string }[] = [];

  for (const api of order) {
    try {
      const songs = await fetchFromSource(api, spec, signal);
      return { api, songs, skipped };
    } catch (e) {
      if ((e as Error).name === 'AbortError') throw e;
      const msg = (e as Error).message || '未知错误';
      // 配置类错误（歌单 ID 格式 / 未配置）换源也没用，直接抛
      if (/尚未配置|应为纯数字|歌曲 ID 列表为空/.test(msg)) throw e;
      skipped.push({ api, error: msg });
    }
  }

  const detail = skipped.map((s) => `· ${s.api}\n    ${s.error}`).join('\n');
  throw new Error(`所有音源都取不到歌单：\n${detail}\n可在设置里更换音源，或稍后重试。`);
}

/** 音源体检：逐个试，返回每个源的真实结果（设置页「测试音源」用） */
export async function diagnose(spec: string, signal?: AbortSignal) {
  const out: { api: string; ok: boolean; count: number; ms: number; error?: string }[] = [];
  for (const api of METING_SOURCES) {
    const t0 = Date.now();
    try {
      const songs = await fetchFromSource(api, spec, signal);
      out.push({ api, ok: true, count: songs.length, ms: Date.now() - t0 });
    } catch (e) {
      if ((e as Error).name === 'AbortError') throw e;
      out.push({ api, ok: false, count: 0, ms: Date.now() - t0, error: (e as Error).message });
    }
  }
  return out;
}
