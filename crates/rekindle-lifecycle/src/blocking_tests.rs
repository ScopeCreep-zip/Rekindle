use super::*;

struct Inline;
impl BlockingWatch for Inline {
    fn inline(&self) -> bool {
        std::thread::current().name() == Some("inline-here")
    }
}

#[test]
fn a_watched_thread_runs_the_work_inline_and_others_spawn() {
    static WATCH: Inline = Inline;
    assert!(watch_blocking(&WATCH));
    assert!(!watch_blocking(&WATCH), "the first watch stays");
    let run = |name: &str| {
        std::thread::Builder::new()
            .name(name.to_string())
            .spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap();
                let caller = std::thread::current().id();
                rt.block_on(run_blocking(move || std::thread::current().id() == caller))
                    .unwrap()
            })
            .unwrap()
            .join()
            .unwrap()
    };
    assert!(run("inline-here"), "inline on the watched thread");
    assert!(!run("elsewhere"), "a blocking thread otherwise");
}
