use anyhow::{Context, Result, bail};
use std::os::windows::process::CommandExt;
use std::process::Command;

const TASK: &str = "Vestigo";
const LEGACY_TASK: &str = "sfs";
const NO_WINDOW: u32 = 0x0800_0000;
const BOM: u16 = 0xfeff;

pub(crate) fn enabled() -> bool {
    schtasks(&["/query", "/tn", TASK]).is_ok()
}

pub(crate) fn adopt_legacy() -> Result<()> {
    if schtasks(&["/query", "/tn", LEGACY_TASK]).is_err() {
        return Ok(());
    }
    set(true)?;
    schtasks(&["/delete", "/tn", LEGACY_TASK, "/f"])
}

pub(crate) fn set(on: bool) -> Result<()> {
    if !on {
        return schtasks(&["/delete", "/tn", TASK, "/f"]);
    }
    let exe = std::env::current_exe().context("finding the executable")?;
    let user = format!(
        "{}\\{}",
        std::env::var("USERDOMAIN").unwrap_or_default(),
        std::env::var("USERNAME").context("reading the user name")?
    );
    let definition = task(&escape(&user), &escape(&exe.to_string_lossy()));
    let file = std::env::temp_dir().join(format!("vestigo-task-{}.xml", std::process::id()));
    let bytes: Vec<u8> = std::iter::once(BOM).chain(definition.encode_utf16()).flat_map(u16::to_le_bytes).collect();
    std::fs::write(&file, bytes).with_context(|| format!("writing {}", file.display()))?;
    let created = schtasks(&["/create", "/tn", TASK, "/xml", &file.to_string_lossy(), "/f"]);
    let _ = std::fs::remove_file(&file);
    created
}

fn schtasks(arguments: &[&str]) -> Result<()> {
    let output =
        Command::new("schtasks.exe").args(arguments).creation_flags(NO_WINDOW).output().context("running schtasks")?;
    if !output.status.success() {
        bail!("schtasks {}: {}", arguments.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn task(user: &str, exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{exe}"</Command>
    </Exec>
  </Actions>
</Task>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_definition_escapes_paths() {
        let definition = task(&escape("PC\\Tom & Jerry"), &escape("C:\\A&B\\vestigo.exe"));
        assert!(definition.contains("<UserId>PC\\Tom &amp; Jerry</UserId>"));
        assert!(definition.contains("<Command>\"C:\\A&amp;B\\vestigo.exe\"</Command>"));
    }
}
