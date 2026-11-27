use super::*;

fn run(css: &str) -> Vec<String> {
    scan(&[("a.css".to_string(), css.to_string())])
}

#[test]
fn a_literal_transition_fails_and_a_token_passes() {
    let hits = run(".a {\n  transition: color 0.15s, background 0.15s;\n}");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].starts_with("a.css:2 "), "{hits:?}");
    assert_eq!(
        run(".a { transition: border-color 80ms ease-out; }").len(),
        1
    );
    assert_eq!(run(".a { transition: opacity; }").len(), 1);
    assert!(run(".a { transition: color var(--motion-duration-fast), background var(--motion-duration-fast); }").is_empty());
    assert!(
        run(".a { transition: filter var(--sec-motion-duration); transition: none; }").is_empty()
    );
}

#[test]
fn an_unguarded_moving_animation_fails() {
    let frames = "@keyframes float { 0% { opacity: 0; transform: translateY(0); } 100% { transform: translateY(-9px); } }\n@keyframes hold { 0% { opacity: 0 } 100% { opacity: 1 } }\n";
    let moving = format!("{frames}.a {{ animation: float 2s ease-out forwards; }}");
    let hits = run(&moving);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(
        hits[0].contains("moves for a reduced-motion user"),
        "{hits:?}"
    );
    let guarded = format!("{frames}@media (prefers-reduced-motion: no-preference) {{ @layer components {{ .a {{ animation: float 2s; }} }} }}");
    assert!(run(&guarded).is_empty());
    let fade = format!("{frames}.a {{ animation: hold 2s linear forwards; }}");
    assert!(run(&fade).is_empty());
    let tokened =
        format!("{frames}.a {{ animation: float var(--motion-duration-base) ease-out; }}");
    assert!(run(&tokened).is_empty());
    assert_eq!(
        run(&format!("{frames}.a {{ animation-duration: 2s; }}")).len(),
        1
    );
    assert!(run(".a { animation: none; }").is_empty());
}
#[test]
fn a_guarded_float_with_a_fading_hold_passes_inside_a_layer() {
    let css = "@layer components {\n  .f { animation: hold 2s linear forwards; }\n  @media (prefers-reduced-motion: no-preference) {\n    .f { animation: float 2s ease-out forwards; }\n  }\n  @keyframes hold { 0% { opacity: 1; } 100% { opacity: 0; } }\n  @keyframes float { 0% { transform: scale(0.6); } 100% { opacity: 0; } }\n}";
    assert!(run(css).is_empty(), "{:?}", run(css));
    let unguarded = css.replace("hold 2s", "float 2s");
    let hits = run(&unguarded);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].starts_with("a.css:2 "), "{hits:?}");
}

#[test]
fn duration_and_delay_longhands_are_checked() {
    assert_eq!(
        run(".a { transition-duration: 150ms; transition-delay: 0.1s; }").len(),
        2
    );
    assert!(run(".a { transition-duration: var(--motion-duration-slow); }").is_empty());
}
