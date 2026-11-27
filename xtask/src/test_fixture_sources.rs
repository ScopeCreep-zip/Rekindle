//! The fixture sources the duplicate passes' tests share (gate hardening
//! item 2): the E4.1 S5 voice-adapter port reproduced from history, an
//! unrelated module, and the list shapes of the triage's false positives.

pub(crate) const S5_DESKTOP_DEPS: &str =
    include_str!("../fixtures/near_duplicates/s5_desktop_deps.rs.fixture");
pub(crate) const S5_HOST_DEPS: &str =
    include_str!("../fixtures/near_duplicates/s5_host_deps.rs.fixture");
pub(crate) const S5_DESKTOP_SETUP: &str =
    include_str!("../fixtures/near_duplicates/s5_desktop_setup.rs.fixture");
pub(crate) const S5_HOST_SETUP: &str =
    include_str!("../fixtures/near_duplicates/s5_host_setup.rs.fixture");
pub(crate) const UNRELATED: &str =
    include_str!("../fixtures/near_duplicates/unrelated_quality_cache.rs.fixture");
pub(crate) const LINT_LIST: &str = include_str!("../fixtures/near_duplicates/lint_list.rs.fixture");
pub(crate) const VARIANT_LIST: &str =
    include_str!("../fixtures/near_duplicates/variant_list.rs.fixture");
pub(crate) const VARIANT_LIST_OTHER: &str =
    include_str!("../fixtures/near_duplicates/variant_list_other.rs.fixture");
