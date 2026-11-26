//! The heads-up display: big centre messages, the feed, health bars, power-ups, the scoreboard and the minimap.

#![allow(deprecated)] // the canvas style setters that take a JsValue work everywhere

use wasm_bindgen::{Clamped, JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlElement, ImageData};

use crate::dom::{self, el, now_s};
use lasertag_shared::Ground;

/// A piece of a feed line: plain text, or a name in its colour.
pub enum Part {
    T(String),
    N(String, String),
}

pub fn t(s: &str) -> Part {
    Part::T(s.to_string())
}
pub fn n(name: &str, color: &str) -> Part {
    Part::N(name.to_string(), color.to_string())
}

pub struct Hud {
    center_hide: Option<f64>,
    feed: Vec<(HtmlElement, f64)>,
    pulses: Vec<(HtmlElement, &'static str, u8)>,
}

impl Hud {
    pub fn new() -> Hud {
        Hud { center_hide: None, feed: Vec::new(), pulses: Vec::new() }
    }

    pub fn center(&mut self, text: &str, sub: &str, ms: f64, color: Option<&str>) {
        let (c, s) = (el("center"), el("sub"));
        c.set_text_content(Some(text));
        dom::style(&c, "color", color.unwrap_or(""));
        s.set_text_content(Some(sub));
        dom::style(&c, "opacity", "1");
        dom::style(&s, "opacity", "1");
        self.center_hide = if ms > 0.0 { Some(now_s() + ms / 1000.0) } else { None };
    }

    pub fn feed(&mut self, parts: Vec<Part>) {
        let d = dom::make("div");
        for p in parts {
            match p {
                Part::T(s) => {
                    let _ = d.append_with_str_1(&s);
                }
                Part::N(name, color) => {
                    let b = dom::make("b");
                    b.set_text_content(Some(&name));
                    dom::style(&b, "color", &color);
                    let _ = d.append_with_node_1(&b);
                }
            }
        }
        let f = el("feed");
        let _ = f.prepend_with_node_1(&d);
        self.feed.insert(0, (d, now_s()));
        while self.feed.len() > 6 {
            if let Some((e, _)) = self.feed.pop() {
                e.remove();
            }
        }
    }

    /// Turn a class on now and off again two frames later (a flash that then fades by CSS transition).
    pub fn pulse(&mut self, e: HtmlElement, class: &'static str) {
        dom::class(&e, class, true);
        self.pulses.push((e, class, 2));
    }

    /// Every frame: fade the centre message and old feed lines, end pulses.
    pub fn step(&mut self) {
        let t = now_s();
        if let Some(h) = self.center_hide {
            if t >= h {
                dom::style(&el("center"), "opacity", "0");
                dom::style(&el("sub"), "opacity", "0");
                self.center_hide = None;
            }
        }
        self.feed.retain(|(e, at)| {
            let age = t - at;
            if age > 6.2 {
                e.remove();
                false
            } else {
                if age > 5.5 {
                    dom::style(e, "opacity", "0");
                }
                true
            }
        });
        self.pulses.retain_mut(|(e, c, n)| {
            if *n == 0 {
                dom::class(e, c, false);
                false
            } else {
                *n -= 1;
                true
            }
        });
    }

    pub fn clear_feed(&mut self) {
        for (e, _) in self.feed.drain(..) {
            e.remove();
        }
    }
}

pub fn ordinal(n: usize) -> String {
    let suf = if n % 10 == 1 && n % 100 != 11 {
        "st"
    } else if n % 10 == 2 && n % 100 != 12 {
        "nd"
    } else if n % 10 == 3 && n % 100 != 13 {
        "rd"
    } else {
        "th"
    };
    format!("{}{}", n, suf)
}

/// A scoreboard row's data.
pub struct Row {
    pub id: u32,
    pub name: String,
    pub color: String,
    pub bot: bool,
    pub cols: Vec<i32>,
}

pub fn board(tbody: &web_sys::Element, rows: &[Row], me: u32) {
    tbody.set_inner_html("");
    for r in rows {
        let tr = dom::make("tr");
        if r.id == me {
            tr.set_class_name("mine");
        }
        let td = dom::make("td");
        let dot = dom::make("span");
        dot.set_class_name("dot");
        dom::style(&dot, "color", &r.color);
        let _ = td.append_with_node_1(&dot);
        let _ = td.append_with_str_1(&r.name);
        if r.bot {
            let b = dom::make("span");
            b.set_class_name("botmark");
            b.set_text_content(Some("BOT"));
            let _ = td.append_with_node_1(&b);
        }
        let _ = tr.append_with_node_1(&td);
        for c in &r.cols {
            let t = dom::make("td");
            t.set_class_name("n");
            t.set_text_content(Some(&c.to_string()));
            let _ = tr.append_with_node_1(&t);
        }
        let _ = tbody.append_with_node_1(&tr);
    }
}

/// The minimap: the ground shaded once into a picture, with everyone drawn on top.
pub struct Mini {
    ctx: CanvasRenderingContext2d,
    base: HtmlCanvasElement,
}

pub struct MiniPerson<'a> {
    pub x: f64,
    pub z: f64,
    pub name: &'a str,
    pub color: &'a str,
}

fn ctx2d(c: &HtmlCanvasElement) -> CanvasRenderingContext2d {
    c.get_context("2d").ok().flatten().expect("2d").dyn_into::<CanvasRenderingContext2d>().expect("2d")
}

impl Mini {
    pub fn new(g: &Ground) -> Mini {
        let mini: HtmlCanvasElement = el("mini").dyn_into().expect("canvas");
        let ctx = ctx2d(&mini);
        let base: HtmlCanvasElement = dom::make("canvas").dyn_into().expect("canvas");
        base.set_width(360);
        base.set_height(360);
        let size = g.size;
        let mut data = vec![0u8; 360 * 360 * 4];
        for py in 0..360 {
            for px in 0..360 {
                let wx = px as f64 / 360.0 * size;
                let wz = py as f64 / 360.0 * size;
                let h = g.at(wx, wz);
                let e = g.at(wx + 4.0, wz) - g.at(wx - 4.0, wz);
                let nn = g.at(wx, wz + 4.0) - g.at(wx, wz - 4.0);
                let sh = (0.55 - e * 0.12 + nn * 0.12).clamp(0.0, 1.0);
                let t = (h / 140.0).min(1.0);
                let k = (py * 360 + px) * 4;
                // JavaScript's Math.round for the creek test, as the old map drew it
                let i = ((wx / g.cell + 0.5).floor()).clamp(0.0, g.n as f64 - 1.0) as usize;
                let j = ((wz / g.cell + 0.5).floor()).clamp(0.0, g.n as f64 - 1.0) as usize;
                if g.wet[j * g.n + i] != 0 {
                    data[k] = 20;
                    data[k + 1] = (150.0 + sh * 60.0) as u8;
                    data[k + 2] = 220;
                } else {
                    data[k] = ((14.0 + 70.0 * t) * (0.4 + sh)).min(255.0) as u8;
                    data[k + 1] = ((16.0 + 10.0 * t) * (0.4 + sh)).min(255.0) as u8;
                    data[k + 2] = ((40.0 + 50.0 * t) * (0.4 + sh)).min(255.0) as u8;
                }
                data[k + 3] = 255;
            }
        }
        if let Ok(img) = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&data), 360, 360) {
            let _ = ctx2d(&base).put_image_data(&img, 0.0, 0.0);
        }
        Mini { ctx, base }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        size: f64,
        pillars: &[(f64, f64, f64)],
        spots: &[(f64, f64, &str)],
        people: &[MiniPerson],
        me: Option<(f64, f64, f64, &str, f64)>,
    ) {
        let c = &self.ctx;
        let s = 360.0 / size;
        let _ = c.draw_image_with_html_canvas_element(&self.base, 0.0, 0.0);
        c.set_fill_style(&JsValue::from_str("#2a2548"));
        for &(x, z, r) in pillars {
            c.begin_path();
            let _ = c.arc(x * s, z * s, (r * s).max(2.0), 0.0, 7.0);
            c.fill();
        }
        for &(x, z, color) in spots {
            c.set_fill_style(&JsValue::from_str(color));
            c.save();
            let _ = c.translate(x * s, z * s);
            let _ = c.rotate(std::f64::consts::FRAC_PI_4);
            c.fill_rect(-5.0, -5.0, 10.0, 10.0);
            c.restore();
        }
        c.set_font("bold 24px Trebuchet MS, sans-serif");
        c.set_text_align("center");
        c.set_text_baseline("bottom");
        for p in people {
            let (x, y) = (p.x * s, p.z * s);
            c.set_fill_style(&JsValue::from_str(p.color));
            c.set_shadow_color(p.color);
            c.set_shadow_blur(10.0);
            c.begin_path();
            let _ = c.arc(x, y, 10.0, 0.0, 7.0);
            c.fill();
            c.set_shadow_blur(0.0);
            c.set_line_width(3.0);
            c.set_stroke_style(&JsValue::from_str("#fff"));
            c.stroke();
            c.set_line_width(4.0);
            c.set_stroke_style(&JsValue::from_str("rgba(0,0,0,.75)"));
            let _ = c.stroke_text(p.name, x, y - 13.0);
            c.set_fill_style(&JsValue::from_str("#fff"));
            let _ = c.fill_text(p.name, x, y - 13.0);
        }
        // you: a view cone, a pulsing ring and a big arrow
        if let Some((mx, mz, yaw, col, half)) = me {
            let (x, y) = (mx * s, mz * s);
            c.save();
            let _ = c.translate(x, y);
            let _ = c.rotate(yaw);
            let _ = c.scale(1.5, 1.5);
            if let Ok(g) = c.create_radial_gradient(0.0, 0.0, 0.0, 0.0, 0.0, 70.0) {
                let _ = g.add_color_stop(0.0, "rgba(255,255,255,.35)");
                let _ = g.add_color_stop(1.0, "rgba(255,255,255,0)");
                c.set_fill_style(&g);
            }
            c.begin_path();
            c.move_to(0.0, 0.0);
            let h = half.min(1.2);
            let _ = c.arc(0.0, 0.0, 70.0, -std::f64::consts::FRAC_PI_2 - h, -std::f64::consts::FRAC_PI_2 + h);
            c.close_path();
            c.fill();
            let pulse = (dom::window().performance().map(|p| p.now()).unwrap_or(0.0) / 900.0) % 1.0;
            c.set_stroke_style(&JsValue::from_str(col));
            c.set_global_alpha(1.0 - pulse);
            c.set_line_width(3.0);
            c.begin_path();
            let _ = c.arc(0.0, 0.0, 10.0 + pulse * 18.0, 0.0, 7.0);
            c.stroke();
            c.set_global_alpha(1.0);
            c.set_fill_style(&JsValue::from_str(col));
            c.set_stroke_style(&JsValue::from_str("#fff"));
            c.set_line_width(3.0);
            c.set_shadow_color(col);
            c.set_shadow_blur(14.0);
            c.begin_path();
            c.move_to(0.0, -16.0);
            c.line_to(11.0, 12.0);
            c.line_to(0.0, 6.0);
            c.line_to(-11.0, 12.0);
            c.close_path();
            c.fill();
            c.stroke();
            c.restore();
            c.set_line_width(4.0);
            c.set_stroke_style(&JsValue::from_str("rgba(0,0,0,.75)"));
            let _ = c.stroke_text("YOU", x, y - 28.0);
            c.set_fill_style(&JsValue::from_str("#fff"));
            let _ = c.fill_text("YOU", x, y - 28.0);
        }
    }
}

/// A name tag: the name in white with a glow in the player's colour, drawn on a small canvas.
pub fn label_canvas(text: &str, color: &str) -> HtmlCanvasElement {
    let c: HtmlCanvasElement = dom::make("canvas").dyn_into().expect("canvas");
    c.set_width(256);
    c.set_height(64);
    let x = ctx2d(&c);
    x.set_font("bold 34px Trebuchet MS, sans-serif");
    x.set_text_align("center");
    x.set_text_baseline("middle");
    x.set_shadow_color(color);
    x.set_shadow_blur(12.0);
    x.set_fill_style(&JsValue::from_str("#fff"));
    let _ = x.fill_text(text, 128.0, 32.0);
    c
}
