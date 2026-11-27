use super::{commit_frame_context, FrameContextCommitError, FrameContextMutation};
use crate::platform::FrameId;
use crate::webdriver::session::Session;

#[test]
fn refuses_to_commit_a_frame_after_the_original_window_is_removed() {
    let mut session = Session::new("child-left".to_string());
    let generation = session.browsing_context_generation;

    let result = commit_frame_context(
        &mut session,
        generation,
        false,
        FrameContextMutation::Push(FrameId::Index(0)),
    );

    assert_eq!(result, Err(FrameContextCommitError::WindowMissing));
    assert!(session.frame_context.is_empty());
}

#[test]
fn prioritizes_a_removed_window_over_a_changed_browsing_context() {
    let mut session = Session::new("child-left".to_string());
    session.browsing_context_generation = 1;

    let result = commit_frame_context(
        &mut session,
        0,
        false,
        FrameContextMutation::Push(FrameId::Index(0)),
    );

    assert_eq!(result, Err(FrameContextCommitError::WindowMissing));
    assert!(session.frame_context.is_empty());
}

#[test]
fn preserves_frame_context_when_top_level_commit_loses_the_selected_window() {
    let mut session = Session::new("child-left".to_string());
    session.frame_context.push(FrameId::Index(0));
    session.frame_context.push(FrameId::Index(1));
    let generation = session.browsing_context_generation;

    let result = commit_frame_context(&mut session, generation, false, FrameContextMutation::Clear);

    assert_eq!(result, Err(FrameContextCommitError::WindowMissing));
    assert!(matches!(
        session.frame_context.as_slice(),
        [FrameId::Index(0), FrameId::Index(1)]
    ));
}

#[test]
fn preserves_frame_context_when_parent_commit_loses_the_selected_window() {
    let mut session = Session::new("child-left".to_string());
    session.frame_context.push(FrameId::Index(0));
    session.frame_context.push(FrameId::Index(1));
    let generation = session.browsing_context_generation;

    let result = commit_frame_context(&mut session, generation, false, FrameContextMutation::Pop);

    assert_eq!(result, Err(FrameContextCommitError::WindowMissing));
    assert!(matches!(
        session.frame_context.as_slice(),
        [FrameId::Index(0), FrameId::Index(1)]
    ));
}

#[test]
fn clears_frame_context_when_switching_to_top_level() {
    let mut session = Session::new("child-left".to_string());
    session.frame_context.push(FrameId::Index(0));
    let generation = session.browsing_context_generation;

    let result = commit_frame_context(&mut session, generation, true, FrameContextMutation::Clear);

    assert_eq!(result, Ok(()));
    assert!(session.frame_context.is_empty());
}

#[test]
fn pops_one_frame_context_when_switching_to_parent() {
    let mut session = Session::new("child-left".to_string());
    session.frame_context.push(FrameId::Index(0));
    session.frame_context.push(FrameId::Index(1));
    let generation = session.browsing_context_generation;

    let result = commit_frame_context(&mut session, generation, true, FrameContextMutation::Pop);

    assert_eq!(result, Ok(()));
    assert!(matches!(
        session.frame_context.as_slice(),
        [FrameId::Index(0)]
    ));
}
