//! One value per own route class, looked up by class: the shape the route
//! owner's slots and its outage tracks share (plan RV2).

use rekindle_records::port::RouteClass;

/// A `T` for the general route and one for the media route.
pub(crate) struct PerClass<T> {
    general: T,
    media: T,
}

impl<T> PerClass<T> {
    /// Each class's value, built by `make`.
    pub(crate) fn new(make: impl Fn(RouteClass) -> T) -> Self {
        Self {
            general: make(RouteClass::General),
            media: make(RouteClass::Media),
        }
    }

    /// The value for `class`.
    pub(crate) fn get(&self, class: RouteClass) -> &T {
        match class {
            RouteClass::General => &self.general,
            RouteClass::Media => &self.media,
        }
    }
}
