use super::*;

fn fixture(index: usize) -> Record {
    serde_json::from_str(
        include_str!("../viewing_history/storage/fixtures/v1.jsonl")
            .lines()
            .nth(index + 1)
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn saved_rows_identify_repository_refs_pins_and_file_without_opening_git() {
    let branches = Entry::new(&fixture(0));
    assert_eq!(branches.title, "example");
    assert_eq!(branches.detail, "Branches");
    assert_eq!(branches.directory, "/repositories/example");
    let comparison = Entry::new(&fixture(1));
    assert_eq!(comparison.title, "example — topic-�");
    assert_eq!(
        comparison.detail,
        "2222222 / 3333333 · vs origin/main (remote)"
    );
    assert!(comparison.tooltip.contains("refs/heads/topic-�"));
    assert!(
        comparison
            .tooltip
            .contains("Base: refs/remotes/origin/main")
    );
    assert!(
        comparison
            .tooltip
            .contains("Head: 2222222222222222222222222222222222222222")
    );
    assert!(comparison.tooltip.contains("File: src/file-�.rs"));
    assert_ne!(
        reference_label(&RefName(b"refs/heads/origin/main".to_vec())),
        reference_label(&RefName(b"refs/remotes/origin/main".to_vec()))
    );
}

#[test]
fn unresolved_rows_keep_the_requested_base_instead_of_inventing_pins() {
    let mut record = fixture(1);
    let Destination::Comparison { comparison, .. } = &mut record.viewed.snapshot.destination else {
        unreachable!()
    };
    *comparison = None;
    let row = Entry::new(&record);
    assert_eq!(row.detail, "Not yet resolved · vs origin/main (remote)");
    assert!(row.tooltip.contains("Base: refs/remotes/origin/main"));
    assert!(!row.tooltip.contains("Head:"));
}
