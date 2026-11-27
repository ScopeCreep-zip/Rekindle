use ratatui::backend::TestBackend;
use ratatui::buffer::Cell;
use ratatui::Terminal;
use rekindle_types::presence::{PresenceSharingPolicy, ShareScope};

use super::{CommandResult, CommunityInfoView, ThemeManager, View};

#[test]
fn community_info_draws_the_location_sharing_policy_for_its_community() {
    let mut view = CommunityInfoView::new("here".into());
    view.on_command_result(CommandResult::CommunityInfoLoaded {
        detail: serde_json::from_value(serde_json::json!({
            "anchor": "here",
            "name": "Here",
            "description": "",
            "creator_pseudonyms": [],
            "created_at": 0,
            "member_count": 1,
            "channels": [],
            "roles": [],
            "our_pseudonym": "self",
            "our_roles": [],
        }))
        .unwrap(),
    })
    .unwrap();
    let theme = ThemeManager::load("rekindle-dark").unwrap();
    let mut terminal = Terminal::new(TestBackend::new(64, 18)).unwrap();
    for (community, share_location, text) in [
        ("elsewhere", ShareScope::Members, "Unknown"),
        ("here", ShareScope::Members, "On"),
        ("here", ShareScope::None, "Off"),
    ] {
        view.on_command_result(CommandResult::CommunityPresencePolicyLoaded {
            community: community.into(),
            policy: PresenceSharingPolicy {
                share_location,
                ..PresenceSharingPolicy::default()
            },
        })
        .unwrap();
        terminal
            .draw(|frame| view.draw(frame, frame.area(), &theme).unwrap())
            .unwrap();
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(Cell::symbol)
            .collect::<String>();
        assert!(screen.contains(&format!("Share location: {text}")));
        assert!(screen.contains("Active channel: unavailable"));
    }
}
