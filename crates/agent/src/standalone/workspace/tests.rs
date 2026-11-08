use std::fs;

use super::*;

fn native(path: &Path) -> PathBuf {
    ResolvedPath::resolve(path).unwrap().native().to_path_buf()
}

#[test]
fn the_working_directory_comes_first_then_its_git_root_and_the_added_directories() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("repository");
    let cwd = repository.join("crates").join("one");
    let added = directory.path().join("added");
    fs::create_dir_all(repository.join(".git")).unwrap();
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&added).unwrap();

    let workspace = StandaloneWorkspace::new(&cwd, std::slice::from_ref(&added)).unwrap();
    assert_eq!(
        workspace.roots().unwrap().roots(),
        [native(&cwd), native(&repository), native(&added)]
    );
    assert_eq!(workspace.project(), native(&repository));
    let search = workspace.search_roots(&TurnExtensions::default()).unwrap();
    assert_eq!(
        search,
        [
            ("repository".to_string(), native(&repository)),
            ("added".to_string(), native(&added)),
        ]
    );
}

#[test]
fn a_workspace_directory_that_is_not_one_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file");
    fs::write(&file, "").unwrap();
    assert!(StandaloneWorkspace::new(&directory.path().join("missing"), &[]).is_err());
    assert!(StandaloneWorkspace::new(directory.path(), &[file]).is_err());
}
