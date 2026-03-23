use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Gui,
    ScanOnce {
        #[arg(long)]
        window: Option<String>,
    },
}

fn main() -> Result<()> {
    #[cfg(windows)]
    if relaunch_as_admin_if_needed()? {
        return Ok(());
    }

    let cli = Cli::parse();

    maa_auto_reverse_rs::bootstrap()?;

    match cli.command.unwrap_or(Command::Gui) {
        Command::Gui => maa_auto_reverse_rs::app::run_gui().map_err(Into::into),
        Command::ScanOnce { window } => {
            let output = maa_auto_reverse_rs::orchestrator::run_scan_once_cli(window)?;
            println!("{output}");
            Ok(())
        }
    }
}

#[cfg(windows)]
fn relaunch_as_admin_if_needed() -> Result<bool> {
    use anyhow::anyhow;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::core::PCWSTR;

    if is_elevated()? {
        return Ok(false);
    }

    let exe = std::env::current_exe()?;
    let params = join_windows_args(std::env::args_os().skip(1));

    let exe_w = to_wide(exe.as_os_str());
    let params_w = to_wide(params.as_os_str());
    let verb_w = to_wide(std::ffi::OsStr::new("runas"));

    let result = unsafe {
        ShellExecuteW(
            Some(HWND(std::ptr::null_mut())),
            PCWSTR(verb_w.as_ptr()),
            PCWSTR(exe_w.as_ptr()),
            PCWSTR(params_w.as_ptr()),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };

    if result.0 as usize <= 32 {
        return Err(anyhow!("请求管理员权限失败"));
    }

    Ok(true)
}

#[cfg(windows)]
fn is_elevated() -> Result<bool> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)? };

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    }?;

    unsafe {
        let _ = CloseHandle(token);
    }

    Ok(elevation.TokenIsElevated != 0)
}

#[cfg(windows)]
fn join_windows_args(args: impl Iterator<Item = std::ffi::OsString>) -> std::ffi::OsString {
    let mut joined = std::ffi::OsString::new();
    let mut first = true;
    for arg in args {
        if !first {
            joined.push(" ");
        }
        first = false;
        joined.push(quote_windows_arg(&arg));
    }
    joined
}

#[cfg(windows)]
fn quote_windows_arg(arg: &std::ffi::OsStr) -> std::ffi::OsString {
    let text = arg.to_string_lossy();
    if !text.contains([' ', '\t', '"']) {
        return arg.to_os_string();
    }

    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for ch in text.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                if backslashes > 0 {
                    out.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                }
                out.push(ch);
            }
        }
    }
    if backslashes > 0 {
        out.push_str(&"\\".repeat(backslashes * 2));
    }
    out.push('"');
    std::ffi::OsString::from(out)
}

#[cfg(windows)]
fn to_wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    value.encode_wide().chain(std::iter::once(0)).collect()
}
