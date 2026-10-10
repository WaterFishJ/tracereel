//! Launch background tools without creating a Windows console window.

use std::ffi::OsStr;
use std::process::Command;

pub(crate) fn background_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Piped stdio alone does not prevent a console from being allocated
        // when FFmpeg is launched by our Windows GUI application.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn child_has_no_console_and_preserves_output_and_exit_status() {
        let output = background_command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", r#"
Add-Type 'using System; using System.Runtime.InteropServices; public static class ConsoleProbe { [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow(); }'
[Console]::Out.WriteLine('console=' + [ConsoleProbe]::GetConsoleWindow().ToInt64())
[Console]::Error.WriteLine('stderr-ok')
exit 7
"#])
            .output()
            .expect("launch console probe");
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "console=0");
        assert!(String::from_utf8_lossy(&output.stderr).contains("stderr-ok"));
    }
}
