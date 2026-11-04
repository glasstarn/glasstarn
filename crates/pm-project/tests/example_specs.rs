//! Every pack's starting extract spec passes review, so a person can begin
//! from it.
use std::path::Path;

#[test]
fn example_specs_pass_review() {
    let packs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs");
    let mut reviewed = 0;
    for entry in std::fs::read_dir(packs).unwrap().flatten() {
        let spec = entry.path().join("extract.spec.example.json");
        let Ok(bytes) = std::fs::read(&spec) else {
            continue;
        };
        pm_project::extract::review(&bytes)
            .unwrap_or_else(|error| panic!("{}: {error}", spec.display()));
        reviewed += 1;
    }
    assert!(reviewed >= 7, "{reviewed}");
}
