//! Creek Laser Tag in the browser, in Rust compiled to WebAssembly.
//!
//! This file starts the game: it loads the ground, builds the app, wires up the page (menus, keyboard, mouse,
//! touch) and runs the frame loop. The game itself is in `game.rs`.

mod audio;
mod dom;
mod game;
mod geom;
mod gl;
mod hud;
mod render;
mod shaders;
mod world;

use std::cell::RefCell;
use std::rc::Rc;

use lasertag_shared::{Ground, GroundMeta, PALETTE};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{AddEventListenerOptions, Element, Event, EventTarget, KeyboardEvent, MouseEvent, TouchEvent};

use dom::el;
use game::App;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Run something with the app (skipped if it is not there yet or already busy).
pub fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| match a.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

fn listen(target: &EventTarget, ev: &str, f: impl FnMut(Event) + 'static) {
    let c = Closure::<dyn FnMut(Event)>::new(f);
    let _ = target.add_event_listener_with_callback(ev, c.as_ref().unchecked_ref());
    c.forget();
}

fn listen_opts(target: &EventTarget, ev: &str, capture: bool, passive: bool, f: impl FnMut(Event) + 'static) {
    let c = Closure::<dyn FnMut(Event)>::new(f);
    let o = AddEventListenerOptions::new();
    o.set_capture(capture);
    o.set_passive(passive);
    let _ = target.add_event_listener_with_callback_and_add_event_listener_options(ev, c.as_ref().unchecked_ref(), &o);
    c.forget();
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, JsValue> {
    let resp: web_sys::Response = JsFuture::from(dom::window().fetch_with_str(url)).await?.dyn_into()?;
    if !resp.ok() {
        return Err(format!("{} {}", url, resp.status()).into());
    }
    let buf = JsFuture::from(resp.array_buffer()?).await?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    wasm_bindgen_futures::spawn_local(async {
        if let Err(e) = run().await {
            let msg = e.as_string().unwrap_or_else(|| format!("{:?}", e));
            dom::log(&format!("laser tag failed to start: {}", msg));
            if let Some(err) = dom::doc().get_element_by_id("err") {
                err.set_text_content(Some(&format!("The game could not start: {}", msg)));
            }
        }
    });
}

async fn run() -> Result<(), JsValue> {
    let meta: GroundMeta = serde_json::from_slice(&fetch_bytes("./terrain.json").await?).map_err(|e| e.to_string())?;
    let raw = fetch_bytes("./terrain.bin").await?;
    let ground = Ground::from_bytes(meta, &raw)?;
    // phones and tablets: on from the start when the device says it is touch-first, and in any case from the first touch
    let nav = dom::window().navigator();
    let touch_first = (dom::matches("(pointer: coarse)") || nav.max_touch_points() > 1)
        && (js_sys::Reflect::has(&dom::window(), &"ontouchstart".into()).unwrap_or(false) || nav.max_touch_points() > 0);
    let canvas: web_sys::HtmlCanvasElement = el("view").dyn_into()?;
    let app = App::new(canvas.clone(), ground, touch_first)?;
    APP.with(|a| *a.borrow_mut() = Some(app));
    menus();
    events(&canvas);
    touch_controls();
    debug_hook();
    with(|a| a.set_touch_mode(a.touch_mode));
    // the frame loop
    let f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
    let g = f.clone();
    *g.borrow_mut() = Some(Closure::new(move || {
        with(|a| a.frame());
        let _ = dom::window().request_animation_frame(f.borrow().as_ref().unwrap().as_ref().unchecked_ref());
    }));
    let _ = dom::window().request_animation_frame(g.borrow().as_ref().unwrap().as_ref().unchecked_ref());
    Ok(())
}

fn menus() {
    let sw = el("swatches");
    let mut color: String = dom::load("color", PALETTE[(dom::random() * PALETTE.len() as f64) as usize % PALETTE.len()].to_string());
    if !PALETTE.contains(&color.as_str()) {
        color = PALETTE[0].to_string();
    }
    with(|a| a.color = color.clone());
    for c in PALETTE {
        let b = dom::make("button");
        let _ = b.set_attribute("type", "button");
        dom::style(&b, "background", c);
        dom::style(&b, "color", c);
        let _ = b.set_attribute("aria-label", &format!("Colour {}", c));
        let _ = b.set_attribute("aria-pressed", if c == color { "true" } else { "false" });
        let me: Element = b.clone().into();
        listen(&b, "click", move |_| {
            with(|a| a.color = c.to_string());
            let kids = el("swatches").children();
            for i in 0..kids.length() {
                if let Some(o) = kids.item(i) {
                    let _ = o.set_attribute("aria-pressed", if o == me { "true" } else { "false" });
                }
            }
        });
        let _ = sw.append_with_node_1(&b);
    }
    let name: web_sys::HtmlInputElement = el("name").unchecked_into();
    name.set_value(&dom::load::<String>("name", String::new()));
    let sens: web_sys::HtmlInputElement = el("sens").unchecked_into();
    sens.set_value(&with(|a| a.sens).unwrap_or(1.0).to_string());
    listen(&sens, "input", move |e| {
        let v: f64 = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).and_then(|i| i.value().parse().ok()).unwrap_or(1.0);
        with(|a| a.sens = v);
        dom::save("sens", &v);
    });
    let glow = el("glow");
    glow.set_text_content(Some(if with(|a| a.glow_on).unwrap_or(true) { "On" } else { "Off" }));
    listen(&glow, "click", move |_| {
        let on = with(|a| {
            let on = !a.glow_on;
            a.set_glow(on);
            on
        })
        .unwrap_or(true);
        el("glow").set_text_content(Some(if on { "On" } else { "Off" }));
    });
    listen(&el("play"), "click", |_| {
        with(|a| a.play());
    });
    listen(&name, "keydown", |e| {
        if let Ok(k) = e.dyn_into::<KeyboardEvent>() {
            if k.key() == "Enter" {
                with(|a| a.play());
            }
        }
    });
    listen(&el("resume"), "click", |_| {
        with(|a| {
            if a.touch_mode {
                a.paused = false;
                dom::hidden("pause", true);
            } else {
                a.lock();
            }
        });
    });
    listen(&el("leave"), "click", |_| {
        with(|a| a.leave());
    });
}

fn locked_on(cv: &web_sys::HtmlCanvasElement) -> bool {
    let c: &Element = cv.as_ref();
    dom::doc().pointer_lock_element().map_or(false, |e| &e == c)
}

fn events(canvas: &web_sys::HtmlCanvasElement) {
    let win: EventTarget = dom::window().into();
    let doc: EventTarget = dom::doc().into();
    listen(&win, "resize", |_| {
        with(|a| a.resize());
    });
    let cv = canvas.clone();
    listen(&doc, "pointerlockchange", move |_| {
        let locked = locked_on(&cv);
        with(|a| {
            if !a.playing {
                return;
            }
            if locked {
                a.paused = false;
                dom::hidden("pause", true);
            } else {
                a.paused = true;
                a.firing = false;
                a.keys.clear();
                dom::hidden("pause", false);
            }
        });
    });
    let cv = canvas.clone();
    listen(&doc, "mousemove", move |e| {
        let Ok(m) = e.dyn_into::<MouseEvent>() else { return };
        let locked = locked_on(&cv);
        with(|a| {
            if !a.playing || a.paused || !locked {
                return;
            }
            a.look(m.movement_x() as f64 * 0.0022, m.movement_y() as f64 * 0.0022);
        });
    });
    let cv = canvas.clone();
    listen(canvas, "mousedown", move |e| {
        let Ok(m) = e.dyn_into::<MouseEvent>() else { return };
        let locked = locked_on(&cv);
        with(|a| {
            if !a.playing {
                return;
            }
            if !locked {
                a.lock();
                return;
            }
            if a.ms.active {
                if m.button() == 0 {
                    a.detonate(None);
                }
                return;
            }
            if m.button() == 0 {
                a.start_firing();
            }
            if m.button() == 2 {
                a.zoom_held = true;
            }
        });
    });
    listen(&win, "mouseup", |e| {
        let Ok(m) = e.dyn_into::<MouseEvent>() else { return };
        with(|a| {
            if m.button() == 0 {
                a.firing = false;
            }
            if m.button() == 2 {
                a.zoom_held = false;
            }
        });
    });
    listen(&win, "contextmenu", |e| {
        if with(|a| a.playing).unwrap_or(false) {
            e.prevent_default();
        }
    });
    listen(&win, "keydown", |e| {
        let Ok(k) = e.dyn_into::<KeyboardEvent>() else { return };
        with(|a| {
            if !a.playing {
                return;
            }
            let code = k.code();
            if code == "Tab" {
                k.prevent_default();
                a.show_board(true);
                return;
            }
            if a.paused {
                return;
            }
            a.keys.insert(code.clone());
            match code.as_str() {
                "Space" => {
                    k.prevent_default();
                    a.jump();
                }
                "KeyZ" => a.zoom_held = true,
                "KeyQ" if !k.repeat() => {
                    a.sfx.wake();
                    if a.ms.active {
                        a.detonate(None)
                    } else {
                        a.launch_missile()
                    }
                }
                "KeyE" if !k.repeat() => {
                    a.sfx.wake();
                    if a.ms.active {
                        a.detonate(None)
                    } else {
                        a.launch_drone()
                    }
                }
                _ => {}
            }
        });
    });
    listen(&win, "keyup", |e| {
        let Ok(k) = e.dyn_into::<KeyboardEvent>() else { return };
        with(|a| {
            let code = k.code();
            a.keys.remove(&code);
            if code == "KeyZ" {
                a.zoom_held = false;
            }
            if code == "Tab" {
                k.prevent_default();
                a.show_board(false);
            }
        });
    });
    listen(&win, "blur", |_| {
        with(|a| {
            a.keys.clear();
            a.firing = false;
            a.zoom_held = false;
        });
    });
    listen_opts(&win, "touchstart", true, true, |_| {
        with(|a| {
            if !a.touch_mode {
                a.set_touch_mode(true);
            }
        });
    });
    listen_opts(&win, "pointerdown", true, false, |e| {
        let Ok(p) = e.dyn_into::<web_sys::PointerEvent>() else { return };
        if p.pointer_type() == "mouse" && !dom::matches("(pointer: coarse)") {
            with(|a| {
                if a.touch_mode {
                    a.set_touch_mode(false);
                }
            });
        }
    });
    listen(&win, "gesturestart", |e| e.prevent_default()); // no pinch-zoom in the middle of a game
    listen(&win, "dblclick", |e| {
        if with(|a| a.touch_mode).unwrap_or(false) {
            e.prevent_default();
        }
    });
}

fn press(a: &mut App, id: &str, tt: &web_sys::Touch) {
    if a.touch_mode {
        dom::window().navigator().vibrate_with_duration(8);
    }
    match id {
        "tfire" => {
            if a.ms.active {
                a.detonate(None);
                return;
            }
            a.start_firing();
            a.touch.fire_id = Some(tt.identifier());
            a.touch.fx = tt.client_x() as f64;
            a.touch.fy = tt.client_y() as f64;
            dom::class(&el("tfire"), "on", true);
        }
        "tmis" | "tdrone" => {
            a.sfx.wake();
            if a.ms.active {
                a.detonate(None)
            } else if id == "tmis" {
                a.launch_missile()
            } else {
                a.launch_drone()
            }
        }
        "tjump" => a.jump(),
        "tzoom" => {
            a.zoom_toggle = !a.zoom_toggle;
            dom::class(&el("tzoom"), "on", a.zoom_toggle);
        }
        "tboard" => a.show_board(dom::is_hidden("board")),
        "tmenu" => {
            a.paused = true;
            a.firing = false;
            dom::hidden("pause", false);
        }
        _ => {}
    }
}

fn stick_rest() {
    let stick = el("stick");
    dom::class(&stick, "on", false);
    dom::class(&stick, "sprint", false);
    dom::style(&stick, "left", "");
    dom::style(&stick, "top", "");
    dom::style(&el("knob"), "transform", "");
}

/// Touch: a stick that appears wherever your left thumb lands, drag anywhere on the right to look (also while
/// holding FIRE), and buttons.
fn touch_controls() {
    let t = el("touch");
    let radius = || {
        let (w, h) = dom::inner_size();
        (w.min(h) * 0.1).clamp(50.0, 85.0)
    };
    let look_k = || 4.2 / dom::inner_size().0.max(320.0); // a swipe across the screen turns you about 240°
    listen_opts(&t, "touchstart", false, false, move |e| {
        let Ok(te) = e.clone().dyn_into::<TouchEvent>() else { return };
        let list = te.changed_touches();
        with(|a| {
            for i in 0..list.length() {
                let Some(tt) = list.get(i) else { continue };
                let btn = tt.target().and_then(|t| t.dyn_into::<Element>().ok()).and_then(|e| e.closest(".tbtn").ok().flatten());
                if let Some(b) = btn {
                    press(a, &b.id(), &tt);
                    continue;
                }
                let (x, y) = (tt.client_x() as f64, tt.client_y() as f64);
                if a.touch.stick_id.is_none() && x < dom::inner_size().0 * 0.45 {
                    a.touch.stick_id = Some(tt.identifier());
                    a.touch.cx = x;
                    a.touch.cy = y;
                    let stick = el("stick");
                    dom::style(&stick, "left", &format!("{}px", x));
                    dom::style(&stick, "top", &format!("{}px", y));
                    dom::class(&stick, "on", true);
                } else if a.touch.look_id.is_none() {
                    a.touch.look_id = Some(tt.identifier());
                    a.touch.lx = x;
                    a.touch.ly = y;
                }
            }
        });
        e.prevent_default();
    });
    listen_opts(&t, "touchmove", false, false, move |e| {
        let Ok(te) = e.clone().dyn_into::<TouchEvent>() else { return };
        let list = te.changed_touches();
        with(|a| {
            for i in 0..list.length() {
                let Some(tt) = list.get(i) else { continue };
                let (x, y) = (tt.client_x() as f64, tt.client_y() as f64);
                let id = Some(tt.identifier());
                if id == a.touch.stick_id {
                    let r = radius();
                    let (mut dx, mut dy) = (x - a.touch.cx, y - a.touch.cy);
                    let mut d = dx.hypot(dy);
                    if d > r {
                        // the stick follows a thumb that slides off it
                        a.touch.cx += dx * (1.0 - r / d);
                        a.touch.cy += dy * (1.0 - r / d);
                        dx *= r / d;
                        dy *= r / d;
                        d = r;
                        let stick = el("stick");
                        dom::style(&stick, "left", &format!("{}px", a.touch.cx));
                        dom::style(&stick, "top", &format!("{}px", a.touch.cy));
                    }
                    dom::style(&el("knob"), "transform", &format!("translate(calc(-50% + {}px), calc(-50% + {}px))", dx, dy));
                    a.touch.mx = dx / r;
                    a.touch.mz = dy / r;
                    dom::class(&el("stick"), "sprint", d > r * 0.92);
                } else if id == a.touch.look_id {
                    let k = look_k();
                    let (dx, dy) = ((x - a.touch.lx) * k, (y - a.touch.ly) * k);
                    a.look(dx, dy);
                    a.touch.lx = x;
                    a.touch.ly = y;
                } else if id == a.touch.fire_id {
                    let k = look_k();
                    let (dx, dy) = ((x - a.touch.fx) * k, (y - a.touch.fy) * k);
                    a.look(dx, dy);
                    a.touch.fx = x;
                    a.touch.fy = y;
                }
            }
        });
        e.prevent_default();
    });
    let end = |e: Event| {
        let Ok(te) = e.dyn_into::<TouchEvent>() else { return };
        let list = te.changed_touches();
        with(|a| {
            for i in 0..list.length() {
                let Some(tt) = list.get(i) else { continue };
                let id = Some(tt.identifier());
                if id == a.touch.stick_id {
                    a.touch.stick_id = None;
                    a.touch.mx = 0.0;
                    a.touch.mz = 0.0;
                    stick_rest();
                }
                if id == a.touch.look_id {
                    a.touch.look_id = None;
                }
                if id == a.touch.fire_id {
                    a.touch.fire_id = None;
                    a.firing = false;
                    dom::class(&el("tfire"), "on", false);
                }
            }
        });
    };
    listen(&t, "touchend", end);
    listen(&t, "touchcancel", end);
}

/// `window.__lt`, for poking at from the console and for the browser tests.
fn debug_hook() {
    let o = js_sys::Object::new();
    let set = |k: &str, v: &JsValue| {
        let _ = js_sys::Reflect::set(&o, &k.into(), v);
    };
    let state = Closure::<dyn Fn() -> String>::new(|| with(|a| a.debug_state()).unwrap_or_default());
    set("state", state.as_ref());
    state.forget();
    let fire = Closure::<dyn Fn(bool)>::new(|on: bool| {
        with(|a| a.firing = on);
    });
    set("fire", fire.as_ref());
    fire.forget();
    let launch = Closure::<dyn Fn()>::new(|| {
        with(|a| a.launch_missile());
    });
    set("launch", launch.as_ref());
    launch.forget();
    let detonate = Closure::<dyn Fn()>::new(|| {
        with(|a| a.detonate(None));
    });
    set("detonate", detonate.as_ref());
    detonate.forget();
    let place = Closure::<dyn Fn(f64, f64, f64, f64)>::new(|x: f64, z: f64, yaw: f64, pitch: f64| {
        with(|a| a.debug_place(x, z, yaw, pitch));
    });
    set("place", place.as_ref());
    place.forget();
    let _ = js_sys::Reflect::set(&dom::window(), &"__lt".into(), &o);
}
