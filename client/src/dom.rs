//! Small helpers for the page: elements by id, classes, styles, the clock and saved settings.

use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlElement, Window};

pub fn window() -> Window {
    web_sys::window().expect("window")
}

pub fn doc() -> Document {
    window().document().expect("document")
}

pub fn el(id: &str) -> HtmlElement {
    doc().get_element_by_id(id).unwrap_or_else(|| panic!("#{} missing", id)).dyn_into::<HtmlElement>().expect("html element")
}

pub fn text(id: &str, s: &str) {
    el(id).set_text_content(Some(s));
}

pub fn class(e: &Element, name: &str, on: bool) {
    let _ = e.class_list().toggle_with_force(name, on);
}

pub fn has(e: &Element, name: &str) -> bool {
    e.class_list().contains(name)
}

pub fn hidden(id: &str, on: bool) {
    class(&el(id), "hidden", on);
}

pub fn is_hidden(id: &str) -> bool {
    has(&el(id), "hidden")
}

pub fn style(e: &HtmlElement, prop: &str, v: &str) {
    let _ = e.style().set_property(prop, v);
}

pub fn make(tag: &str) -> HtmlElement {
    doc().create_element(tag).expect("create").dyn_into::<HtmlElement>().expect("html element")
}

/// Seconds, from the page's high-resolution clock.
pub fn now_s() -> f64 {
    window().performance().map(|p| p.now()).unwrap_or(0.0) / 1000.0
}

pub fn inner_size() -> (f64, f64) {
    let w = window();
    (
        w.inner_width().ok().and_then(|v| v.as_f64()).unwrap_or(800.0),
        w.inner_height().ok().and_then(|v| v.as_f64()).unwrap_or(600.0),
    )
}

pub fn device_pixel_ratio() -> f64 {
    window().device_pixel_ratio()
}

pub fn matches(q: &str) -> bool {
    window().match_media(q).ok().flatten().map(|m| m.matches()).unwrap_or(false)
}

/// Settings saved in the browser, under the same keys the old page used ("lt.name" and so on), as JSON.
pub fn load<T: serde::de::DeserializeOwned>(k: &str, d: T) -> T {
    let Some(s) = window().local_storage().ok().flatten() else { return d };
    match s.get_item(&format!("lt.{}", k)) {
        Ok(Some(v)) => serde_json::from_str(&v).unwrap_or(d),
        _ => d,
    }
}

pub fn save<T: serde::Serialize>(k: &str, v: &T) {
    if let Some(s) = window().local_storage().ok().flatten() {
        if let Ok(j) = serde_json::to_string(v) {
            let _ = s.set_item(&format!("lt.{}", k), &j);
        }
    }
}

pub fn random() -> f64 {
    js_sys::Math::random()
}

pub fn log(s: &str) {
    web_sys::console::log_1(&s.into());
}
