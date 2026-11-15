//! Platform-related code
use daedalus::minecraft::{Os, OsRule};

/// (e.g. "Run as administrator" on Windows).
#[cfg(target_os = "windows")]
pub fn is_process_elevated() -> bool {
    use std::ffi::c_void;
    use std::ptr::null_mut;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(
            process_handle: *mut c_void,
            desired_access: u32,
            token_handle: *mut *mut c_void,
        ) -> i32;
        fn GetTokenInformation(
            token_handle: *mut c_void,
            token_information_class: u32,
            token_information: *mut c_void,
            token_information_length: u32,
            return_length: *mut u32,
        ) -> i32;
    }

    const TOKEN_QUERY: u32 = 0x8;
    const TOKEN_ELEVATION_TYPE: u32 = 18;
    const TOKEN_ELEVATION_TYPE_FULL: u32 = 2;

    // SAFETY: GetCurrentProcess returns a pseudo-handle that must not be
    // closed. The token handle is queried and closed on every path.
    unsafe {
        let process = GetCurrentProcess();
        let mut token = null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &raw mut token) == 0
            || token.is_null()
        {
            return false;
        }

        let mut elevation_type: u32 = 0;
        let mut size = 0u32;
        let success = GetTokenInformation(
            token,
            TOKEN_ELEVATION_TYPE,
            &raw mut elevation_type as *mut c_void,
            std::mem::size_of::<u32>() as u32,
            &raw mut size,
        ) != 0;
        let _ = CloseHandle(token);
        success && elevation_type == TOKEN_ELEVATION_TYPE_FULL
    }
}

#[cfg(not(target_os = "windows"))]
pub fn is_process_elevated() -> bool {
    false
}

// Bit width
#[cfg(target_pointer_width = "64")]
pub const ARCH_WIDTH: &str = "64";

#[cfg(target_pointer_width = "32")]
pub const ARCH_WIDTH: &str = "32";

// Platform rule handling
fn normalize_architecture(java_arch: &str) -> &str {
    if java_arch.eq_ignore_ascii_case("amd64") {
        "x86_64"
    } else if java_arch.eq_ignore_ascii_case("i386")
        || java_arch.eq_ignore_ascii_case("i686")
    {
        "x86"
    } else if java_arch.eq_ignore_ascii_case("arm64") {
        "aarch64"
    } else if java_arch.eq_ignore_ascii_case("arm32") {
        "arm"
    } else {
        java_arch
    }
}

pub fn os_rule(
    rule: &OsRule,
    java_arch: &str,
    // Minecraft updated over 1.18.2 (supports MacOS Natively)
    _minecraft_updated: bool,
) -> bool {
    let mut rule_match = true;
    let normalized_arch = normalize_architecture(java_arch);

    if let Some(expected) = rule.arch.as_deref() {
        let actual = normalized_arch.to_ascii_lowercase();
        rule_match &= match expected.to_ascii_lowercase().as_str() {
            "x86" | "i386" | "i686" => {
                matches!(actual.as_str(), "x86" | "i386" | "i686")
            }
            "x86_64" | "amd64" => {
                matches!(actual.as_str(), "x86_64" | "amd64")
            }
            "arm" | "arm32" => actual.starts_with("arm") && actual != "arm64",
            "aarch64" | "arm64" => {
                matches!(actual.as_str(), "aarch64" | "arm64")
            }
            expected => expected == actual,
        };
    }

    if let Some(name) = &rule.name {
        let native = Os::native_arch(normalized_arch);
        rule_match &= name == &Os::Universal
            || name == &native
            || name == &native.get_os();
    }

    // `rule.version` is ignored because it's not usually seen on real recent
    // Minecraft version manifests, its alleged regex syntax is undefined and is
    // likely to not match `Regex`'s, and the way to get the value to match it
    // against is allegedly calling `System.getProperty("os.version")`, which
    // on Windows the OpenJDK implements by fetching the kernel32.dll version,
    // an approach that no public Rust library implements. Moreover, launchers
    // such as PrismLauncher also ignore this field. Code references:
    // - https://github.com/openjdk/jdk/blob/948ade8e7003a41683600428c8e3155c7ed798db/src/java.base/windows/native/libjava/java_props_md.c#L556
    // - https://github.com/PrismLauncher/PrismLauncher/blob/1c20faccf88999474af70db098a4c10e7a03af33/launcher/minecraft/Rule.h#L77
    // - https://github.com/FillZpp/sys-info-rs/blob/60ecf1470a5b7c90242f429934a3bacb6023ec4d/c/windows.c#L23-L38

    rule_match
}

pub fn classpath_separator(java_arch: &str) -> &'static str {
    match Os::native_arch(java_arch) {
        Os::Osx
        | Os::OsxArm64
        | Os::Linux
        | Os::LinuxArm32
        | Os::LinuxArm64
        | Os::Universal
        | Os::Unknown => ":",
        Os::Windows | Os::WindowsArm64 => ";",
    }
}

/// 以管理员身份重新启动自己（Windows）。
///
/// 为什么要这个而不是清单里写 `requireAdministrator`：
/// 实测两种清单写法都会让 exe 启动时报「应用程序的并行配置不正确」——
/// 自定义清单资源与 linker 自动生成的清单撞车，连只含 trustInfo 的最小清单也炸。
/// 链接器开关 `/MANIFESTUAC` 又被 rust-lld 忽略（字节流里查不到 requestedExecutionLevel）。
/// 所以改成运行期自检：不是管理员就 runas 自己一次，然后退出。
///
/// 防循环：调用方在重启前设置 `HAYENAI_ELEVATED_RELAUNCH=1`，子进程继承环境变量，
/// 看到它就不再重启（万一 runas 失败也不会无限弹 UAC）。
#[cfg(target_os = "windows")]
pub fn relaunch_elevated() -> Result<(), String> {
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{
        SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows::core::PCWSTR;

    let mut executable = std::env::current_exe()
        .map_err(|error| format!("could not locate executable: {error}"))?
        .into_os_string()
        .encode_wide()
        .collect::<Vec<_>>();
    executable.push(0);

    // 原样带上启动参数（例如 --memory-optimize 这类 helper 参数）
    let arguments = std::env::args_os()
        .skip(1)
        .map(|argument| format!("\"{}\"", argument.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    let mut arguments_w: Vec<u16> = arguments.encode_utf16().collect();
    arguments_w.push(0);

    let verb = "runas\0".encode_utf16().collect::<Vec<_>>();

    let mut execute_info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR::from_raw(verb.as_ptr()),
        lpFile: PCWSTR::from_raw(executable.as_ptr()),
        lpParameters: PCWSTR::from_raw(arguments_w.as_ptr()),
        nShow: 1, // SW_SHOWNORMAL
        ..Default::default()
    };

    unsafe { ShellExecuteExW(&raw mut execute_info) }
        .map_err(|error| format!("could not relaunch as administrator: {error}"))?;
    Ok(())
}
