use super::*;

#[test]
fn test_parse_strategy() {
    assert_eq!(
        LocatorStrategy::from_string("css selector"),
        Some(LocatorStrategy::CssSelector)
    );
    assert_eq!(
        LocatorStrategy::from_string("xpath"),
        Some(LocatorStrategy::XPath)
    );
    assert_eq!(LocatorStrategy::from_string("unknown"), None);
}

#[test]
fn test_css_selector_js() {
    let strategy = LocatorStrategy::CssSelector;
    let js = strategy.to_selector_js("#my-button");

    assert!(js.contains("querySelector"));
    assert!(js.contains("#my-button"));
}

#[test]
fn test_xpath_js() {
    let strategy = LocatorStrategy::XPath;
    let js = strategy.to_selector_js("//div[@id='test']");

    assert!(js.contains("document.evaluate"));
    // Single quotes are escaped in the output
    assert!(js.contains("//div[@id=\\'test\\']"));
}

#[test]
fn test_escaping() {
    let strategy = LocatorStrategy::CssSelector;
    let js = strategy.to_selector_js("div[data-value='test']");

    assert!(js.contains("div[data-value=\\'test\\']"));
}
