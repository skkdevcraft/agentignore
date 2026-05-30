use agentignore::fs::HandleTable;

use fuser::FileHandle;

use std::fs;

mod common;

#[test]
fn handle_table_insert_and_get() {
    let (_dir, root) = common::test_dir();
    let path = root.join("test.txt");
    common::touch(&path);

    let mut table = HandleTable::new();
    let f = fs::File::open(&path).unwrap();
    let fh = table.insert(f);

    assert!(table.get(fh).is_some());
}

#[test]
fn handle_table_insert_increments() {
    let (_dir, root) = common::test_dir();
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    common::touch(&a);
    common::touch(&b);

    let mut table = HandleTable::new();
    let fha = table.insert(fs::File::open(&a).unwrap());
    let fhb = table.insert(fs::File::open(&b).unwrap());
    assert_ne!(fha.0, fhb.0);
    assert!(fhb.0 > fha.0);
}

#[test]
fn handle_table_remove() {
    let (_dir, root) = common::test_dir();
    let path = root.join("test.txt");
    common::touch(&path);

    let mut table = HandleTable::new();
    let fh = table.insert(fs::File::open(&path).unwrap());
    table.remove(fh);
    assert!(table.get(fh).is_none());
}

#[test]
fn handle_table_get_unknown() {
    let table = HandleTable::new();
    assert!(table.get(FileHandle(999)).is_none());
}

#[test]
fn handle_table_remove_non_existent_is_noop() {
    let mut table = HandleTable::new();
    table.remove(FileHandle(42));
}
