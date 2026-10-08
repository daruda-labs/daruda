//! Check Codex's native dependency before an ACP session consumes its pipes.

use std::path::Path;
use std::time::Duration;

use super::{NpmAdapter, runtime::NpmRuntime};
use crate::preparation::{PreparationContext, PreparationError, PreparationKind};

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const CODEX_PROBE: &str = concat!(
    "const {createRequire}=require('node:module');",
    "const {dirname,join}=require('node:path');",
    "const {spawnSync}=require('node:child_process');",
    "const req=createRequire(process.argv[1]);",
    "const manifest=req.resolve('@openai/codex/package.json');",
    "const cli=join(dirname(manifest),'bin','codex.js');",
    "const child=spawnSync(process.execPath,[cli,'--version'],",
    "{encoding:'utf8',windowsHide:true,timeout:10000,maxBuffer:65536});",
    "if(child.error){console.error(child.error.message);process.exit(1);}",
    "process.stdout.write(child.stdout||'');process.stderr.write(child.stderr||'');",
    "process.exit(child.status??1);"
);

pub(super) fn verify(
    adapter: &NpmAdapter,
    runtime: &NpmRuntime,
    entry: &Path,
    context: &PreparationContext<'_>,
) -> Result<(), PreparationError> {
    if adapter.bin != "codex-acp" {
        return Ok(());
    }
    let mut command = runtime.command();
    command.args(["-e", CODEX_PROBE]).arg(entry);
    let version = crate::preparation::process::output_with_timeout(&mut command, context, PROBE_TIMEOUT)
        .map_err(|error| PreparationError::new(error.kind, format!(
            "Codex native CLI launch check failed (adapter: {}, path units: {}): {}; check runtime path length, file access, and installed architecture",
            entry.display(), entry.as_os_str().len(), error.detail,
        )))?;
    if !version.trim_start().starts_with("codex-cli ") {
        return Err(PreparationError::new(
            PreparationKind::InvalidPackage,
            "Codex native CLI launch check returned an unexpected version response",
        ));
    }
    Ok(())
}
