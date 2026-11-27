use crate::platform::executor::*;

#[test]
fn converts_physical_window_rect_at_common_scale_factors() {
    let cases = [
        (
            PhysicalPosition::new(-200, 150),
            PhysicalSize::new(720, 540),
            1.0,
            WindowRect {
                x: -200,
                y: 150,
                width: 720,
                height: 540,
            },
        ),
        (
            PhysicalPosition::new(-151, 251),
            PhysicalSize::new(1000, 750),
            1.25,
            WindowRect {
                x: -121,
                y: 201,
                width: 800,
                height: 600,
            },
        ),
        (
            PhysicalPosition::new(-400, 300),
            PhysicalSize::new(1440, 1080),
            2.0,
            WindowRect {
                x: -200,
                y: 150,
                width: 720,
                height: 540,
            },
        ),
    ];

    for (position, size, scale_factor, expected) in cases {
        let actual = physical_window_rect_to_logical(position, size, scale_factor);

        assert_eq!(actual.x, expected.x);
        assert_eq!(actual.y, expected.y);
        assert_eq!(actual.width, expected.width);
        assert_eq!(actual.height, expected.height);
    }
}

#[test]
fn converts_physical_window_chrome_before_subtracting_it() {
    let retina_chrome = physical_window_chrome_to_logical(
        PhysicalSize::new(1440, 1080),
        PhysicalSize::new(1440, 1032),
        2.0,
    );
    let fractional_chrome = physical_window_chrome_to_logical(
        PhysicalSize::new(1251, 1001),
        PhysicalSize::new(1000, 750),
        1.25,
    );

    assert_eq!(retina_chrome, LogicalSize::new(0, 24));
    assert_eq!(fractional_chrome, LogicalSize::new(201, 201));
}
