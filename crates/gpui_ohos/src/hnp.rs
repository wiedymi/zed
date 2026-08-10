use std::{ffi::OsString, path::PathBuf};

use anyhow::{Result, bail, ensure};

const ZED_TOOLS_PACKAGE: &str = "zedtools";
const ZED_TOOLS_VERSION: &str = "0.7.0";
const PROCESS_PROBE: &str = "zed-hnp-probe";
const SYSTEM_SHELL: &str = "sh";
const GIT: &str = "git";
const NODE: &str = "node";
const NPM: &str = "npm";
const NPX: &str = "npx";

pub fn packaged_executable(name: &str) -> Result<PathBuf> {
    if name != PROCESS_PROBE
        && name != SYSTEM_SHELL
        && name != GIT
        && name != NODE
        && name != NPM
        && name != NPX
    {
        bail!("auxiliary executable {name:?} is not packaged for HarmonyOS");
    }

    let private_home =
        std::env::var_os("HNP_PRIVATE_HOME").unwrap_or_else(|| OsString::from("/data/app"));
    let private_home = PathBuf::from(private_home);
    ensure!(
        private_home.is_absolute(),
        "HarmonyOS supplied a relative HNP_PRIVATE_HOME: {private_home:?}"
    );
    let executable = private_home
        .join(format!("{ZED_TOOLS_PACKAGE}.org"))
        .join(format!("{ZED_TOOLS_PACKAGE}_{ZED_TOOLS_VERSION}"))
        .join("bin")
        .join(name);
    ensure!(
        executable.is_file(),
        "packaged HarmonyOS executable is missing at {}",
        executable.display()
    );
    Ok(executable)
}
