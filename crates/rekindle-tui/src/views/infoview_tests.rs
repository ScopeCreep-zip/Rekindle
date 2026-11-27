use super::*;
use rekindle_types::dto::infoview::InfoviewSection;

fn view(bio: &str) -> InfoviewView {
    InfoviewView {
        subject: InfoviewSubject::Own,
        sections: vec![InfoviewSection::Bio { text: bio.into() }],
    }
}

fn pushed(window: &str, bio: &str) -> SubscriptionEvent {
    SubscriptionEvent::System(SystemEvent::Infoview(InfoviewEvent::Updated {
        window: window.into(),
        view: view(bio),
    }))
}

/// The Host's answer is the page: each section's heading, then its
/// rows.
#[test]
fn the_answer_renders_the_hosts_sections() {
    let mut page = InfoviewPage::new(InfoviewSubject::Own);
    page.on_command_result(CommandResult::InfoviewLoaded { view: view("hi") })
        .unwrap();
    assert_eq!(
        page.lines,
        vec![
            InfoviewLine::Heading("Bio".into()),
            InfoviewLine::Row("hi".into())
        ]
    );
}

/// A push to the TUI's window replaces the page; one to another window
/// (the CLI's) leaves it.
#[test]
fn only_a_push_to_the_tui_window_replaces_the_page() {
    let mut page = InfoviewPage::new(InfoviewSubject::Own);
    page.on_subscription_event(&pushed(TUI_WINDOW, "first"))
        .unwrap();
    page.on_subscription_event(&pushed(rekindle_types::dto::infoview::CLI_WINDOW, "cli"))
        .unwrap();
    assert_eq!(page.lines[1], InfoviewLine::Row("first".into()));
    page.on_subscription_event(&pushed(TUI_WINDOW, "second"))
        .unwrap();
    assert_eq!(page.lines[1], InfoviewLine::Row("second".into()));
}
