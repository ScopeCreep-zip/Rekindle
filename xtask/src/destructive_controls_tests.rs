use super::*;

const GOOD: &str = r#"const D = () => {
  const [s] = createSignal<string>("");
  return (
    <form onSubmit={submit}>
      <button type="button" class="sec-btn-secondary" onClick={() => void cancel()}>Cancel</button>
      <Show when={warning()} fallback={<button type="submit" class="sec-btn-commit">{label()}</button>}>
        <button type="button" class="sec-btn-commit-danger" data-role="destructive" onClick={commit}>
          {label()}
        </button>
      </Show>
    </form>
  );
};"#;

#[test]
fn the_reviewed_shape_passes() {
    let hits = scan("D.tsx", GOOD);
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn a_swapped_class_fails() {
    let swap = r#"<button type="submit" class={warning() ? "form-btn-danger" : "form-btn-primary"}>{label()}</button>"#;
    let hits = scan("D.tsx", swap);
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits[0].contains("chosen by an expression"));
}

#[test]
fn a_default_unlabelled_or_repeated_destructive_control_fails() {
    let default = r#"<button type="submit" class="sec-btn-commit-danger" data-role="destructive">Delete</button>"#;
    assert!(scan("D.tsx", default)[0].contains("default"));
    let focused = r#"<button autofocus class="sec-btn-commit-danger" data-role="destructive">Delete</button>"#;
    assert!(scan("D.tsx", focused)[0].contains("focus"));
    let bare = r#"<button class="sec-btn-commit-danger" data-role="destructive" />"#;
    assert!(scan("D.tsx", bare)[0].contains("non-colour cue"));
    // An accessible name alone, or an icon a sighted user cannot read
    // as a verb, is no cue while no glyph is reserved.
    let named = r#"<button class="sec-btn-commit-danger" data-role="destructive" aria-label="Delete channel" />"#;
    assert!(scan("D.tsx", named)[0].contains("visible verb label"));
    let icon = r#"<button class="sec-btn-commit-danger" data-role="destructive" aria-label="Delete"><span class="nf-icon" aria-hidden="true">{ICON_TRASH}</span></button>"#;
    assert!(scan("D.tsx", icon)[0].contains("visible verb label"));
    let two = format!("{default}{default}").replace("type=\"submit\" ", "");
    assert!(scan("D.tsx", &two)
        .iter()
        .any(|h| h.contains("2 destructive controls")));
}

#[test]
fn a_mark_without_styling_or_styling_without_mark_fails() {
    let unmarked = r#"<button class="context-menu-item-danger">Remove</button>"#;
    assert!(scan("D.tsx", unmarked)[0].contains("without `data-role"));
    let unstyled = r#"<button class="sec-btn-commit" data-role="destructive">Remove</button>"#;
    assert!(scan("D.tsx", unstyled)[0].contains("without its destructive class"));
}
