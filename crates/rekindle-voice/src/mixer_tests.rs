use super::*;

#[test]
fn test_mix_two_streams() {
    let mixer = AudioMixer::new(1);
    let a = [0.5f32, 0.3, -0.2];
    let b = [0.2f32, -0.1, 0.4];
    let result = mixer.mix(&[("a", &a), ("b", &b)]);
    assert_eq!(result.len(), 3);
    assert!((result[0] - 0.7).abs() < 0.001);
    assert!((result[1] - 0.2).abs() < 0.001);
    assert!((result[2] - 0.2).abs() < 0.001);
}

#[test]
fn test_mix_clamps() {
    let mixer = AudioMixer::new(1);
    let a = [0.9f32];
    let b = [0.9f32];
    let result = mixer.mix(&[("a", &a), ("b", &b)]);
    assert!((result[0] - 1.0).abs() < f32::EPSILON); // clamped
}
