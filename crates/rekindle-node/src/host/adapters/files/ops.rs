//! The attachment operations the Host answers on both modes (plan E3.1
//! E3d): upload a file the shell picked, download to the place the shell
//! chose, the record a download dialog names, a voice message's send and
//! its bytes, the chunks a peer asks us for, and the eager expression
//! fetch. The file the Host reads is bounded before it is read: an
//! absolute path to a regular file of at most `MAX_FILE_SIZE_BYTES`.

use std::path::Path;

use rekindle_files::{AttachmentRecordJson, FilesError, MAX_FILE_SIZE_BYTES};

use super::HostFiles;

/// Why a path a request names is refused.
#[derive(Debug, PartialEq, Eq)]
pub enum PathRefusal {
    /// Not an absolute path.
    Relative,
    /// Nothing is there, or it cannot be read.
    Missing,
    /// A directory, a link or a device, not a regular file.
    NotAFile,
    /// Larger than the attachment cap.
    TooLarge { actual: u64 },
    /// The download's folder does not exist.
    NoFolder,
}

impl std::fmt::Display for PathRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Relative => write!(f, "the path must be absolute"),
            Self::Missing => write!(f, "no readable file is at the path"),
            Self::NotAFile => write!(f, "the path is not a regular file"),
            Self::TooLarge { actual } => {
                write!(
                    f,
                    "the file is {actual} bytes, over the {MAX_FILE_SIZE_BYTES}-byte cap"
                )
            }
            Self::NoFolder => write!(f, "the download's folder does not exist"),
        }
    }
}

/// The file an upload reads: absolute, a regular file (a link is not
/// followed), at most `MAX_FILE_SIZE_BYTES`.
pub fn upload_source(path: &Path) -> Result<(), PathRefusal> {
    if !path.is_absolute() {
        return Err(PathRefusal::Relative);
    }
    let meta = std::fs::symlink_metadata(path).map_err(|_| PathRefusal::Missing)?;
    if !meta.file_type().is_file() {
        return Err(PathRefusal::NotAFile);
    }
    if meta.len() > MAX_FILE_SIZE_BYTES {
        return Err(PathRefusal::TooLarge { actual: meta.len() });
    }
    Ok(())
}

/// The place a download writes: absolute, in an existing folder, and not
/// a directory or a link.
pub fn download_target(path: &Path) -> Result<(), PathRefusal> {
    if !path.is_absolute() {
        return Err(PathRefusal::Relative);
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return Err(PathRefusal::NoFolder);
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() => Err(PathRefusal::NotAFile),
        _ => Ok(()),
    }
}

impl HostFiles {
    /// Upload the file at `path` to `channel_id`: its attachment id.
    ///
    /// # Errors
    /// The path is refused, or the upload failed.
    pub async fn upload(
        &self,
        community_id: &str,
        channel_id: &str,
        path: &Path,
    ) -> Result<String, FilesError> {
        upload_source(path).map_err(|e| FilesError::InvalidInput(e.to_string()))?;
        rekindle_files::upload_file(self, community_id, channel_id, path).await
    }

    /// Download `attachment_id_hex` of `channel_id` to `dest`.
    ///
    /// # Errors
    /// The place is refused, or the download failed.
    pub async fn download(
        &self,
        community_id: &str,
        channel_id: &str,
        attachment_id_hex: &str,
        dest: &Path,
    ) -> Result<(), FilesError> {
        download_target(dest).map_err(|e| FilesError::InvalidInput(e.to_string()))?;
        rekindle_files::download_attachment(self, community_id, channel_id, attachment_id_hex, dest)
            .await
    }

    /// The stored record of an attachment, for the shell's dialogs.
    ///
    /// # Errors
    /// The rows could not be read.
    pub async fn info(
        &self,
        channel_id: &str,
        attachment_id_hex: &str,
    ) -> Result<Option<AttachmentRecordJson>, FilesError> {
        Ok(self
            .record(channel_id, attachment_id_hex)
            .await?
            .map(|(_, record)| record))
    }

    /// An attachment's plaintext, fetched into the chunk cache if needed
    /// (a voice message plays from it; nothing is written outside the
    /// cache).
    ///
    /// # Errors
    /// The fetch or the assembly failed.
    pub async fn bytes(
        &self,
        community_id: &str,
        channel_id: &str,
        attachment_id_hex: &str,
    ) -> Result<Vec<u8>, FilesError> {
        let fetched = rekindle_files::fetch_attachment_to_cache(
            self,
            community_id,
            channel_id,
            attachment_id_hex,
        )
        .await?;
        rekindle_files::assemble_attachment(self, community_id, &fetched)
    }

    /// Send a recorded voice message: its attachment id.
    ///
    /// # Errors
    /// The upload failed.
    pub async fn voice(
        &self,
        community_id: &str,
        channel_id: &str,
        opus: Vec<u8>,
        duration_ms: u32,
        waveform: Vec<u8>,
    ) -> Result<String, FilesError> {
        rekindle_files::send_voice_message_bytes(
            self,
            community_id,
            channel_id,
            opus,
            duration_ms,
            waveform,
        )
        .await
    }

    /// Fetch every expression asset of `community_id` not yet cached, so a
    /// member holds the emoji bytes it renders.
    pub async fn eager_fetch(&self, community_id: &str) {
        rekindle_files::eager_fetch_missing(self, community_id).await;
    }
}

/// Answer a peer's `RequestAttachment`: the chunks we hold of
/// `attachment_id`, from the community cache that holds it, or `None`.
/// Who asks is not checked: a chunk is ciphertext under the file key,
/// which only a member holds.
pub fn serve(
    store: &rekindle_files::FileCacheStore,
    attachment_id: [u8; 16],
    requested_chunks: &[u32],
) -> Option<Vec<u8>> {
    let community = store.holder_of(uuid::Uuid::from_bytes(attachment_id))?;
    store.serve(&community, attachment_id, requested_chunks)
}

#[cfg(test)]
mod tests {
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
}
