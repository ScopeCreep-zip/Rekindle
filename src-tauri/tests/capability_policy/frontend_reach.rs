//! What a window's frontend code reaches: the commands its own files call
//! and those behind the stores, actions and components it imports (plan
//! C7.28). Split from `capability_policy.rs` by size; its
//! `windows_are_granted_the_commands_their_code_reaches` and
//! `prop_variants_still_call_commands` tests use it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::manifest_dir;
use crate::window_labels::WindowKind;

/// The frontend code a window owns: its entry module and, when it has one,
/// its own folder under `src/windows/`. Shared modules (stores, actions,
/// hydration) import nearly every command wrapper, so the import closure
/// cannot tell which commands a window really calls; the code it owns can
/// (plan C7.28).
pub(crate) fn owned_frontend_code(kind: WindowKind) -> (&'static str, Option<&'static str>) {
    match kind {
        WindowKind::Login => ("src/windows/LoginWindow.tsx", None),
        WindowKind::BuddyList => ("src/windows/BuddyListWindow.tsx", None),
        WindowKind::Settings => (
            "src/windows/SettingsWindow.tsx",
            Some("src/windows/settings"),
        ),
        WindowKind::Chat => (
            "src/windows/ChatWindow.tsx",
            Some("src/windows/chat_window"),
        ),
        WindowKind::Community => (
            "src/windows/CommunityWindow.tsx",
            Some("src/windows/community_window"),
        ),
        WindowKind::Call => ("src/windows/CallWindow.tsx", None),
        WindowKind::Profile => ("src/windows/ProfileWindow.tsx", None),
        WindowKind::SkinEditor => (
            "src/windows/SkinEditorWindow.tsx",
            Some("src/windows/skin_editor"),
        ),
    }
}

pub(crate) fn repo_root() -> PathBuf {
    manifest_dir()
        .parent()
        .expect("src-tauri has a parent")
        .canonicalize()
        .expect("the repo root resolves")
}

pub(crate) fn frontend_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            frontend_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "ts" || x == "tsx") {
            out.push(path);
        }
    }
}

/// `commands.<wrapper>` → the command it invokes, from `src/ipc/commands/`.
pub(crate) fn command_wrappers() -> BTreeMap<String, String> {
    // `invoke` and `hostInvoke` (a Host call the shell forwards, E7.12): a
    // case-sensitive `invoke\b` alone missed every `hostInvoke(` wrapper.
    let invoke =
        regex::Regex::new(r#"\b(?:invoke|hostInvoke)\b[^"\n]*?\(\s*"(\w+)""#).expect("regex");
    let key = regex::Regex::new(r"(?m)^\s*(\w+)\s*[:(]").expect("regex");
    let mut files = Vec::new();
    frontend_files(&repo_root().join("src/ipc/commands"), &mut files);
    let mut map = BTreeMap::new();
    for file in files {
        let src = std::fs::read_to_string(&file).expect("readable");
        for m in invoke.captures_iter(&src) {
            let at = m.get(0).expect("match").start();
            if let Some(k) = key.captures_iter(&src[..at]).last() {
                map.insert(k[1].to_owned(), m[1].to_owned());
            }
        }
    }
    map
}

/// Resolve a relative TS import to its file.
fn resolve_import(from: &Path, spec: &str) -> Option<PathBuf> {
    let base = from.parent()?.join(spec);
    ["", ".ts", ".tsx", "/index.ts", "/index.tsx"]
        .iter()
        .map(|ext| PathBuf::from(format!("{}{ext}", base.display())))
        .find(|p| p.is_file())
        .map(|p| p.canonicalize().expect("an import resolves"))
}

/// A module's relative value imports: (file, imported names). A default
/// import is named `default`; `import type` brings no code.
fn module_imports(file: &Path, src: &str) -> Vec<(PathBuf, Vec<String>)> {
    let named = regex::Regex::new(
        r#"(?s)import\s+(type\s+)?(\w+\s*,\s*)?\{([^}]*)\}\s*from\s*"(\.[^"]+)""#,
    )
    .expect("regex");
    let default = regex::Regex::new(r#"import\s+(\w+)\s+from\s*"(\.[^"]+)""#).expect("regex");
    let mut out = Vec::new();
    for m in named.captures_iter(src) {
        if m.get(1).is_some() {
            continue;
        }
        let names = m[3]
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty() && !n.starts_with("type "))
            .map(|n| n.split_whitespace().next().unwrap_or(n).to_owned())
            .collect();
        if let Some(path) = resolve_import(file, &m[4]) {
            out.push((path, names));
        }
    }
    for m in default.captures_iter(src) {
        if &m[1] == "type" {
            continue;
        }
        if let Some(path) = resolve_import(file, &m[2]) {
            out.push((path, vec!["default".to_owned()]));
        }
    }
    out
}

/// A module's top-level functions: name → source up to the next one.
fn module_functions(src: &str) -> BTreeMap<String, String> {
    let start = regex::Regex::new(
        r"(?m)^(?:export\s+)?(?:async\s+)?(?:function\s+(\w+)|const\s+(\w+)\s*=\s*(?:async\s*)?(?:\(|\w+\s*=>))",
    )
    .expect("regex");
    let starts: Vec<(usize, String)> = start
        .captures_iter(src)
        .map(|m| {
            let name = m.get(1).or_else(|| m.get(2)).expect("a name");
            (m.get(0).expect("match").start(), name.as_str().to_owned())
        })
        .collect();
    let mut out = BTreeMap::new();
    for (i, (at, name)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(src.len(), |(next, _)| *next);
        out.insert(name.clone(), src[*at..end].to_owned());
    }
    out
}

/// What a window's code reaches: the commands its own files call, and the
/// commands behind every store function its files or the components they
/// mount (transitively) import, followed through that function's own calls
/// into other stores and actions. A store hook such as
/// `trackNetworkStatus()` invokes a command in every window that mounts it,
/// so the window must be granted it (walk 20 M3: the community window's
/// Create waited on a status read the ACL refused). A mounted component's
/// own command calls count too (walk 21: the chat window's link preview and
/// Settings' skin credits called `open_external_url` ungranted), except in
/// [`PROP_VARIANTS`], whose calls sit behind the props that say which
/// window's variant it renders.
pub(crate) struct Reach {
    pub(crate) commands: BTreeSet<(String, PathBuf)>,
    files: BTreeSet<PathBuf>,
    functions: BTreeSet<(PathBuf, String)>,
    sources: BTreeMap<PathBuf, String>,
}

fn is_under(path: &Path, dir: &str) -> bool {
    path.starts_with(repo_root().join(dir))
}

/// Shared components that render one variant per window from their props:
/// the video panel's DM, community and native-preview paths
/// (`useVideoCallPanel.ts`, `panel_capture.ts`) each call only their own
/// variant's commands, so a window is granted its variant's, not every
/// branch's. `prop_variants_still_call_commands` keeps each entry live.
pub(crate) const PROP_VARIANTS: [&str; 1] = ["src/components/voice/video_call"];

fn is_prop_variant(path: &Path) -> bool {
    PROP_VARIANTS.iter().any(|dir| is_under(path, dir))
}

impl Reach {
    pub(crate) fn new() -> Self {
        Self {
            commands: BTreeSet::new(),
            files: BTreeSet::new(),
            functions: BTreeSet::new(),
            sources: BTreeMap::new(),
        }
    }

    pub(crate) fn source(&mut self, path: &Path) -> String {
        self.sources
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            })
            .clone()
    }

    pub(crate) fn scan_calls(&mut self, at: &Path, src: &str, wrappers: &BTreeMap<String, String>) {
        let call = regex::Regex::new(r"\bcommands\s*\.\s*(\w+)\s*\(").expect("regex");
        for m in call.captures_iter(src) {
            if let Some(cmd) = wrappers.get(&m[1]) {
                self.commands.insert((cmd.clone(), at.to_path_buf()));
            }
        }
    }

    pub(crate) fn file(
        &mut self,
        path: &Path,
        owned: &dyn Fn(&Path) -> bool,
        w: &BTreeMap<String, String>,
    ) {
        if !self.files.insert(path.to_path_buf()) {
            return;
        }
        let src = self.source(path);
        if owned(path) || (is_under(path, "src/components") && !is_prop_variant(path)) {
            self.scan_calls(path, &src, w);
        }
        for (dep, names) in module_imports(path, &src) {
            if owned(&dep) || is_under(&dep, "src/components") {
                self.file(&dep, owned, w);
            } else if is_under(&dep, "src/stores") {
                for name in names {
                    self.function(&dep, &name, w);
                }
            }
        }
    }

    fn function(&mut self, path: &Path, name: &str, w: &BTreeMap<String, String>) {
        if !self.functions.insert((path.to_path_buf(), name.to_owned())) {
            return;
        }
        let src = self.source(path);
        let functions = module_functions(&src);
        let Some(body) = functions.get(name) else {
            return;
        };
        self.scan_calls(path, body, w);
        let imports = module_imports(path, &src);
        let ident = regex::Regex::new(r"\b(\w+)\s*\(").expect("regex");
        for m in ident.captures_iter(body) {
            let callee = &m[1];
            if callee != name && functions.contains_key(callee) {
                self.function(path, callee, w);
            } else if let Some((dep, _)) = imports.iter().find(|(dep, names)| {
                (is_under(dep, "src/stores") || is_under(dep, "src/actions"))
                    && names.iter().any(|n| n == callee)
            }) {
                self.function(&dep.clone(), callee, w);
            }
        }
    }
}
