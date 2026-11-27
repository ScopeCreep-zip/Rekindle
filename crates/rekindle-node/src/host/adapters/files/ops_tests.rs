use super::{download_target, upload_source, PathRefusal};
use rekindle_files::MAX_FILE_SIZE_BYTES;

#[test]
fn an_upload_reads_only_an_absolute_regular_file_under_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.png");
    std::fs::write(&file, b"png").unwrap();
    assert_eq!(upload_source(&file), Ok(()));
    assert_eq!(
        upload_source(std::path::Path::new("a.png")),
        Err(PathRefusal::Relative)
    );
    assert_eq!(
        upload_source(&dir.path().join("none")),
        Err(PathRefusal::Missing)
    );
    assert_eq!(upload_source(dir.path()), Err(PathRefusal::NotAFile));
    #[cfg(unix)]
    {
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(upload_source(&link), Err(PathRefusal::NotAFile));
    }
    let big = dir.path().join("big.bin");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(MAX_FILE_SIZE_BYTES + 1).unwrap();
    assert_eq!(
        upload_source(&big),
        Err(PathRefusal::TooLarge {
            actual: MAX_FILE_SIZE_BYTES + 1
        })
    );
    let edge = dir.path().join("edge.bin");
    std::fs::File::create(&edge)
        .unwrap()
        .set_len(MAX_FILE_SIZE_BYTES)
        .unwrap();
    assert_eq!(upload_source(&edge), Ok(()));
}

#[test]
fn a_download_writes_only_a_file_in_an_existing_folder() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(download_target(&dir.path().join("new.png")), Ok(()));
    assert_eq!(
        download_target(std::path::Path::new("new.png")),
        Err(PathRefusal::Relative)
    );
    assert_eq!(
        download_target(&dir.path().join("no/such/x.png")),
        Err(PathRefusal::NoFolder)
    );
    assert_eq!(download_target(dir.path()), Err(PathRefusal::NotAFile));
}
