//! Writer-facing setup prepares only this installation's agent bundle.
use super::super::{
    DaemonRuntime,
    http::{Request, Response},
};
use serde_json::json;

pub(crate) fn prepare(_: &Request, runtime: &DaemonRuntime) -> Response {
    if crate::agent_plugins::generate(&runtime.config).is_err() {
        return Response::json(
            500,
            &json!({"error":"Could not prepare the connection files. Check that Hieronymus can write to its settings folder."}),
        );
    }
    let root = runtime.config.agent_plugins_root();
    let agents = [("codex", "Codex", "codex"), ("cowork", "Cowork", "claude"), ("pi", "pi", "pi")]
        .map(|(id, name, directory)| {
            let package = root.join(directory);
            let instructions = format!(
                "Set up Hieronymus for this writing project in {name}: (1) add its MCP connection, and (2) install its skills for working with Hieronymus. Hieronymus stores several types of agent memory for writing projects.\n\nThe installation-owned plugin catalog is at {}. The {name} package is at {}. The data root is {}. Use your supported plugin or package installation flow, preserving my other settings. The generated package includes the MCP connection and writing skills; pi additionally requires pi-mcp-adapter. A custom data root must be passed through HIERONYMUS_DATA_ROOT. Never copy or print credentials.\n\nCheck that you can access this local installation from your environment before making changes. If you cannot, explain the supported connection step I need to take; do not claim that the connection works. If your host requires permission to enable hooks, ask me through its normal trust interface.\n\nVerify both setup steps separately: confirm that Hieronymus MCP tools respond and that the installed Hieronymus skills are discoverable by this agent. Then inspect this project's context, and follow its instructions and my memory agreement. Recognizing the project is not permission to import the whole book. Tell me whether the MCP connection and the skills were each verified; report any incomplete step. Once both are ready, continue writing or translating here.",
                root.display(), package.display(), runtime.config.data_root().display());
            json!({"id":id,"name":name,"instructions":instructions})
        });
    Response::json(200, &json!({"agents":agents}))
}
