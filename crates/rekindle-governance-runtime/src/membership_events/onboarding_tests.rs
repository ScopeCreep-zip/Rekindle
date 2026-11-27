use super::*;
use rekindle_types::governance::OnboardingOption;

fn option(option_id: &str, roles: &[u8]) -> OnboardingOption {
    OnboardingOption {
        option_id: option_id.to_string(),
        title: option_id.to_string(),
        description: None,
        emoji: None,
        roles_to_assign: roles.iter().map(|&n| RoleId([n; 16])).collect(),
        channels_to_show: Vec::new(),
    }
}

fn question(
    question_id: &str,
    required: bool,
    single_select: bool,
    options: Vec<OnboardingOption>,
) -> OnboardingQuestion {
    OnboardingQuestion {
        question_id: question_id.to_string(),
        title: question_id.to_string(),
        description: None,
        required,
        single_select,
        options,
    }
}

fn answer(question_id: &str, selected: &[&str]) -> OnboardingAnswer {
    OnboardingAnswer {
        question_id: question_id.to_string(),
        selected_options: selected.iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn collects_roles_from_selected_options() {
    let questions = vec![question(
        "q1",
        true,
        false,
        vec![option("a", &[1]), option("b", &[2, 3])],
    )];
    let roles = validate_onboarding_answers(&questions, &[answer("q1", &["a", "b"])])
        .expect("valid answers");
    let expected: HashSet<RoleId> = [RoleId([1; 16]), RoleId([2; 16]), RoleId([3; 16])].into();
    assert_eq!(roles, expected);
}

#[test]
fn rejects_unknown_question() {
    let questions = vec![question("q1", false, false, vec![option("a", &[1])])];
    assert_eq!(
        validate_onboarding_answers(&questions, &[answer("q2", &["a"])]),
        Err(OnboardingRejection::UnknownQuestion {
            question_id: "q2".to_string()
        })
    );
}

#[test]
fn rejects_required_question_omitted() {
    let questions = vec![question("q1", true, false, vec![option("a", &[1])])];
    assert_eq!(
        validate_onboarding_answers(&questions, &[]),
        Err(OnboardingRejection::RequiredOmitted {
            question_id: "q1".to_string()
        })
    );
}

#[test]
fn rejects_required_question_with_empty_selection() {
    let questions = vec![question("q1", true, false, vec![option("a", &[1])])];
    assert_eq!(
        validate_onboarding_answers(&questions, &[answer("q1", &[])]),
        Err(OnboardingRejection::RequiredEmpty {
            question_id: "q1".to_string()
        })
    );
}

#[test]
fn rejects_single_select_with_multiple_selections() {
    let questions = vec![question(
        "q1",
        false,
        true,
        vec![option("a", &[1]), option("b", &[2])],
    )];
    assert_eq!(
        validate_onboarding_answers(&questions, &[answer("q1", &["a", "b"])]),
        Err(OnboardingRejection::SingleSelectMultiple {
            question_id: "q1".to_string()
        })
    );
}

#[test]
fn rejects_unknown_option() {
    let questions = vec![question("q1", false, false, vec![option("a", &[1])])];
    assert_eq!(
        validate_onboarding_answers(&questions, &[answer("q1", &["zzz"])]),
        Err(OnboardingRejection::UnknownOption {
            question_id: "q1".to_string(),
            option_id: "zzz".to_string(),
        })
    );
}

#[test]
fn optional_unanswered_question_is_skipped() {
    let questions = vec![
        question("q1", false, false, vec![option("a", &[1])]),
        question("q2", false, false, vec![option("b", &[2])]),
    ];
    let roles = validate_onboarding_answers(&questions, &[answer("q2", &["b"])])
        .expect("optional q1 may be skipped");
    assert_eq!(roles, [RoleId([2; 16])].into());
}
