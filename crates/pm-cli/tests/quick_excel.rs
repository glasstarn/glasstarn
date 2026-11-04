//! The quick one-file commands read an Excel workbook the way a project
//! import does, with the same result as the same data saved as CSV.
use std::process::Command;

fn summary(file: &str) -> serde_json::Value {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(["analyze", repo.join(file).to_str().unwrap(), "--json"])
        .env("GLASSTARN_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn analyze_reads_a_workbook_like_its_csv() {
    let workbook = summary("testdata/journeys/claims.xlsx");
    let csv = summary("testdata/journeys/claims.csv");
    assert_eq!(workbook["summary"]["cases"], 1200);
    assert_eq!(workbook["summary"], csv["summary"]);
    assert_eq!(workbook["variants"], csv["variants"]);
    // Named as the person gave it, not as the converted copy.
    assert!(
        workbook["source"]
            .as_str()
            .unwrap()
            .ends_with("claims.xlsx")
    );
}
