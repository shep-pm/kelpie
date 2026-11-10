//! Each harness's own files in a worktree, which the sandbox's fence keeps a
//! worker from changing, and which each adapter supplies
//!
//! `harness::OWN_FILES`, named by `sandbox::fence`

use crate::harness::OWN_FILES;
use crate::sandbox::fence;

use super::*;

#[test]
fn every_harness_supplies_its_own_files_to_the_fence() {
    let supplied: Vec<&str> = OWN_FILES.iter().map(|own| own.harness).collect();
    assert_eq!(supplied, ["Claude Code", "goose"]);

    // The fence names what each supplies, in the case a file system would read it in
    for (file, owner) in [
        (".claude/settings.json", "Claude Code"),
        (".mcp.json", "Claude Code"),
        (".goose/config.yaml", "goose"),
        (".CONFIG/goose/settings.json", "goose"),
    ] {
        assert_eq!(fence::owner(Path::new(file)), Some(owner), "{file}");
    }

    // A harness with no adapter yet keeps its row in the fence, so a worker on
    // any harness still cannot plant its files
    assert_eq!(fence::owner(Path::new(".codex/config.toml")), Some("Codex"));
    assert_eq!(fence::owner(Path::new(".pi/extensions/x.ts")), Some("pi"));
}
