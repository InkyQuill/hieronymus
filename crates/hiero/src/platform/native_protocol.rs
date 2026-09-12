//! The broker's two-phase authority handoff, independent of native API bindings.
use std::io::{BufRead, Write};
/// Guards outlive every mutation/readback/rollback. Readiness is not permission;
/// truncated input or EOF before the exact commit token never calls `operation`.
pub fn committed<R: BufRead, W: Write, G, T>(
    input: &mut R,
    output: &mut W,
    guards: G,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _guards = guards;
    output
        .write_all(b"READY\n")
        .and_then(|_| output.flush())
        .map_err(|_| "Native readiness delivery failed")?;
    let mut commit = String::new();
    std::io::Read::take(&mut *input, 8)
        .read_line(&mut commit)
        .map_err(|_| "Native commit input failed")?;
    if commit != "COMMIT\n" {
        return Err("Native operation was not committed".into());
    }
    operation()
}
