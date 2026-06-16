//! File content search overlay state (Ctrl+G).

#[derive(Clone, Debug)]
pub struct FileSearchResult {
    pub file_path: String,
    pub line_number: usize,
    pub line_content: String,
}

/// File content search with query, scored results, and selection.
#[derive(Debug)]
pub struct FileSearchState {
    pub query: String,
    pub results: Vec<FileSearchResult>,
    pub selected_index: Option<usize>,
    pub total_matches: usize,
    pub visible: bool,
}

impl FileSearchState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            selected_index: None,
            total_matches: 0,
            visible: true,
        }
    }
}

impl Default for FileSearchState {
    fn default() -> Self {
        Self::new()
    }
}
