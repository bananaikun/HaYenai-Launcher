//! `.tex` 体检：**只读文件头**，回答「这个包需要 GPU 的哪项能力」。
//!
//! 为什么客户端要自己算：渲染器对「贴图传不上去」是**静默**的 ——
//! 不报错、`onError` 不触发，用户只看到「背景空白 + 调设置没反应」。
//! `webwallgl` 2.1.0 的 `compressedFormatFor()`（见架构文档 §11.3）对 ETC2
//! （format 3/5）与 BPTC（12）**不查扩展**就直接取常量，在 ANGLE-D3D11
//! （WebView2 的形态，只给 S3TC）上 `internalFormat` 是 `undefined` ⇒
//! 整张贴图被拒；而软件解码器又不认 3/5/12 ⇒ 背景整块消失。
//!
//! 这里只读头、不解码、不转码（转码是二期的事）。每个文件固定读 64 字节。
//!
//! 刻意**不**回答「这台机器画不画得出」：同一份包在不同设备上答案不同，
//! 前半段（包需要什么）在这里算，后半段（设备有没有）留给前端。

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::{Result, TexAudit, WallpaperError};

/// `.tex` 容器 magic（9 字节，末尾带 NUL）。
const TEX_MAGIC: &[u8; 9] = b"TEXV0005\0";
/// 第二段 magic，紧跟其后。
const TEX_IMAGE_MAGIC: &[u8; 9] = b"TEXI0001\0";
/// magic 之后到 format/flags 的偏移：两个 magic 各 9 字节。
const FORMAT_OFFSET: usize = 18;
/// 读取量：格式与 flags 都在这 64 字节里，其余字段（宽高、mip 数）用不到。
pub const HEAD_BYTES: usize = 64;
/// 目录递归深度上限，防御异常的深层目录。
const MAX_DEPTH: usize = 32;

/// 贴图格式「画出来要靠什么」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TexFamily {
    /// ETC2（format 3/5）：只有 GPU 一条路，且库里不查扩展。
    Etc2,
    /// S3TC / DXT（format 4/6/7）：库里有软件解码兜底。
    Dxt,
    /// BPTC / BC7（format 12）：只有 GPU 一条路，软件解码直接抛「暂未实现」。
    Bptc,
    /// 未压缩或库能软件解码的格式（0/1/2/8/9/10/11/13/14/15）。
    Plain,
    /// 谁都不认（库的分支里没有）。
    Unknown,
}

impl TexFamily {
    /// 格式名，与 `wetex.cjs` 的 `FORMATS` 表同名，便于和服务端日志对照。
    pub fn format_name(format: u32) -> &'static str {
        match format {
            0 => "ARGB8888",
            1 => "RGB888",
            2 => "RGB565",
            3 => "ETC2_RGB8",
            4 => "DXT5",
            5 => "ETC2_RGBA8",
            6 => "DXT3",
            7 => "DXT1",
            8 => "RG88",
            9 => "R8",
            10 => "RG1616f",
            11 => "R16f",
            12 => "BC7",
            13 => "RGBa1010102",
            14 => "RGBA16161616f",
            15 => "RGB161616f",
            _ => "unknown",
        }
    }
}

/// 按 format 数值判族。取值照 `webwallgl` 的真实分支，不是照 README。
pub fn family_of(format: u32) -> TexFamily {
    match format {
        3 | 5 => TexFamily::Etc2,
        4 | 6 | 7 => TexFamily::Dxt,
        12 => TexFamily::Bptc,
        0 | 1 | 2 | 8 | 9 | 10 | 11 | 13 | 14 | 15 => TexFamily::Plain,
        _ => TexFamily::Unknown,
    }
}

/// 文件头解析结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexHead {
    pub format: u32,
    pub flags: u32,
}

/// 从任意 `Read` 里读头（只读 [`HEAD_BYTES`] 字节，单测用计数 reader 验证）。
///
/// 返回 `None` 表示「头读不懂」：文件太短、magic 不是 `TEXV0005`、
/// 或不是 `TEXI0001` 段。这类文件一律计入 `unreadable`，不静默放过。
pub fn read_head(reader: &mut impl Read) -> Option<TexHead> {
    let mut header = [0_u8; HEAD_BYTES];
    let mut filled = 0;
    while filled < HEAD_BYTES {
        match reader.read(&mut header[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    if filled < FORMAT_OFFSET + 8 {
        return None;
    }
    if &header[..TEX_MAGIC.len()] != TEX_MAGIC {
        return None;
    }
    if &header[TEX_MAGIC.len()..TEX_MAGIC.len() + TEX_IMAGE_MAGIC.len()]
        != TEX_IMAGE_MAGIC
    {
        return None;
    }
    let format = u32::from_le_bytes([
        header[FORMAT_OFFSET],
        header[FORMAT_OFFSET + 1],
        header[FORMAT_OFFSET + 2],
        header[FORMAT_OFFSET + 3],
    ]);
    let flags = u32::from_le_bytes([
        header[FORMAT_OFFSET + 4],
        header[FORMAT_OFFSET + 5],
        header[FORMAT_OFFSET + 6],
        header[FORMAT_OFFSET + 7],
    ]);
    Some(TexHead { format, flags })
}

/// 读一个文件头。打不开或读不懂都返回 `None`。
pub fn read_head_of_file(path: &Path) -> Option<TexHead> {
    let mut file = File::open(path).ok()?;
    read_head(&mut file)
}

/// 扫一个目录下所有 `.tex`，统计格式分布。
///
/// 单个文件读失败只影响计数（进 `unreadable`），不会让整次导入失败 ——
/// 贴图体检失败最多是「少一条告警」，绝不能因此装不进来。
pub fn audit_dir(root: &Path) -> Result<TexAudit> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_tex(root, &mut files, 0)?;

    let mut audit = TexAudit::default();
    for path in files {
        audit.total += 1;
        let Some(head) = read_head_of_file(&path) else {
            audit.unreadable += 1;
            continue;
        };
        let name = TexFamily::format_name(head.format).to_string();
        *audit.formats.entry(name).or_insert(0) += 1;
        match family_of(head.format) {
            TexFamily::Etc2 => audit.etc2 = true,
            TexFamily::Dxt => audit.dxt = true,
            TexFamily::Bptc => audit.bptc = true,
            TexFamily::Plain => {}
            // 库的分支里没有的格式：谁都不认，当作「可能画不出来」。
            TexFamily::Unknown => audit.unreadable += 1,
        }
    }
    Ok(audit)
}

/// 递归收集 `.tex`（扩展名大小写不敏感，符号链接不跟随）。
fn collect_tex(
    dir: &Path,
    out: &mut Vec<PathBuf>,
    depth: usize,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let read_dir = match std::fs::read_dir(dir) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(error) => return Err(WallpaperError::Io(error)),
    };

    for entry in read_dir {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_tex(&path, out, depth + 1)?;
            continue;
        }
        let is_tex = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("tex"));
        if is_tex {
            out.push(path);
        }
    }
    Ok(())
}

/// 汇总成给界面看的「格式 → 张数」文本，日志与告警都用得上。
pub fn describe(audit: &TexAudit) -> String {
    if audit.total == 0 {
        return "没有 .tex 贴图".to_string();
    }
    let mut parts: Vec<String> = audit
        .formats
        .iter()
        .map(|(name, count)| format!("{name}×{count}"))
        .collect();
    if audit.unreadable > 0 {
        parts.push(format!("读不出×{}", audit.unreadable));
    }
    parts.join("、")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// 拼一个 64 字节以上的假 `.tex`：two magics + format + flags + 垃圾。
    fn tex_bytes(format: u32, flags: u32, total_len: usize) -> Vec<u8> {
        let mut bytes = vec![0_u8; total_len.max(HEAD_BYTES)];
        bytes[..9].copy_from_slice(TEX_MAGIC);
        bytes[9..18].copy_from_slice(TEX_IMAGE_MAGIC);
        bytes[FORMAT_OFFSET..FORMAT_OFFSET + 4]
            .copy_from_slice(&format.to_le_bytes());
        bytes[FORMAT_OFFSET + 4..FORMAT_OFFSET + 8]
            .copy_from_slice(&flags.to_le_bytes());
        bytes
    }

    /// 计数 reader：记录被读走的字节数，用来证明「只读头」。
    struct CountingReader<'a> {
        inner: Cursor<&'a [u8]>,
        read: usize,
    }

    impl Read for CountingReader<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = self.inner.read(buffer)?;
            self.read += read;
            Ok(read)
        }
    }

    #[test]
    fn detects_etc2_formats() {
        assert_eq!(family_of(3), TexFamily::Etc2);
        assert_eq!(family_of(5), TexFamily::Etc2);
        assert_eq!(TexFamily::format_name(3), "ETC2_RGB8");
        assert_eq!(TexFamily::format_name(5), "ETC2_RGBA8");
    }

    #[test]
    fn detects_dxt_and_bptc_formats() {
        for format in [4, 6, 7] {
            assert_eq!(family_of(format), TexFamily::Dxt);
        }
        assert_eq!(family_of(12), TexFamily::Bptc);
        assert_eq!(TexFamily::format_name(12), "BC7");
    }

    #[test]
    fn plain_and_unknown_formats_are_told_apart() {
        for format in [0, 1, 2, 8, 9, 10, 11, 13, 14, 15] {
            assert_eq!(family_of(format), TexFamily::Plain);
        }
        assert_eq!(family_of(16), TexFamily::Unknown);
        assert_eq!(TexFamily::format_name(16), "unknown");
    }

    #[test]
    fn reads_a_head_from_a_large_file_in_only_64_bytes() {
        let bytes = tex_bytes(5, 0, 1024 * 1024);
        let mut reader = CountingReader {
            inner: Cursor::new(bytes.as_slice()),
            read: 0,
        };

        let head = read_head(&mut reader).expect("head parses");

        assert_eq!(head.format, 5);
        assert_eq!(head.flags, 0);
        assert_eq!(
            reader.read, HEAD_BYTES,
            "只允许读 {} 字节，实际读了 {}",
            HEAD_BYTES, reader.read
        );
        assert!(reader.read < 4096, "上限是每文件 < 4 KB");
    }

    #[test]
    fn rejects_files_with_a_foreign_magic() {
        let mut bytes = tex_bytes(5, 0, HEAD_BYTES);
        bytes[..9].copy_from_slice(b"TEXV0004\0");
        assert!(read_head(&mut Cursor::new(bytes.as_slice())).is_none());

        let mut bytes = tex_bytes(5, 0, HEAD_BYTES);
        bytes[9..18].copy_from_slice(b"XXXX0001\0");
        assert!(read_head(&mut Cursor::new(bytes.as_slice())).is_none());

        // 太短：连 format 字段都放不下。
        let short = b"TEXV0005\0".to_vec();
        assert!(read_head(&mut Cursor::new(short.as_slice())).is_none());

        let empty: Vec<u8> = Vec::new();
        assert!(read_head(&mut Cursor::new(empty.as_slice())).is_none());
    }

    #[test]
    fn audits_a_directory_by_family() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(temp.path().join("materials")).expect("mkdir");
        std::fs::write(
            temp.path().join("materials/a.tex"),
            tex_bytes(5, 0, 128),
        )
        .expect("write etc2");
        std::fs::write(temp.path().join("b.TEX"), tex_bytes(7, 0, 128))
            .expect("write dxt1");
        std::fs::write(temp.path().join("c.tex"), tex_bytes(12, 0, 128))
            .expect("write bc7");
        std::fs::write(temp.path().join("d.tex"), tex_bytes(0, 0, 128))
            .expect("write argb");
        std::fs::write(temp.path().join("broken.tex"), b"nope")
            .expect("write broken");
        // 非 .tex 不参与统计。
        std::fs::write(temp.path().join("preview.jpg"), b"\xff\xd8\xff")
            .expect("write preview");

        let audit = audit_dir(temp.path()).expect("audit");

        assert!(audit.etc2, "format 5 必须报 etc2");
        assert!(audit.dxt, "format 7 必须报 dxt");
        assert!(audit.bptc, "format 12 必须报 bptc");
        assert_eq!(audit.total, 5);
        assert_eq!(audit.unreadable, 1);
        assert_eq!(audit.formats.get("ETC2_RGBA8"), Some(&1));
        assert_eq!(audit.formats.get("DXT1"), Some(&1));
        assert_eq!(audit.formats.get("BC7"), Some(&1));
    }

    #[test]
    fn a_directory_without_tex_audits_clean() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(temp.path().join("project.json"), b"{}").expect("write");

        let audit = audit_dir(temp.path()).expect("audit");

        assert_eq!(audit, TexAudit::default());
        assert!(audit.warning().is_none());
        assert_eq!(describe(&audit), "没有 .tex 贴图");
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        let audit = audit_dir(Path::new("D:/not/here/wallpaper"))
            .expect("missing dir audits clean");
        assert_eq!(audit.total, 0);
    }

    #[test]
    fn describe_lists_each_family() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(temp.path().join("a.tex"), tex_bytes(3, 0, 128))
            .expect("write");
        std::fs::write(temp.path().join("b.tex"), tex_bytes(3, 0, 128))
            .expect("write");

        let audit = audit_dir(temp.path()).expect("audit");

        assert_eq!(describe(&audit), "ETC2_RGB8×2");
    }
}
