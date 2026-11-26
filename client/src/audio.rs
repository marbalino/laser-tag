//! Sound, synthesised on the fly (no files): tones and filtered noise through Web Audio.

use web_sys::{AudioBuffer, AudioContext, AudioContextState, BiquadFilterType, GainNode, OscillatorType};

#[derive(Default)]
pub struct Sfx {
    ac: Option<AudioContext>,
    master: Option<GainNode>,
    noise: Option<AudioBuffer>,
}

#[derive(Clone, Copy)]
pub struct Tone {
    pub f0: f32,
    pub f1: f32,
    pub dur: f64,
    pub ty: OscillatorType,
    pub vol: f32,
    pub pan: f32,
    pub delay: f64,
}

impl Default for Tone {
    fn default() -> Tone {
        Tone { f0: 800.0, f1: 200.0, dur: 0.15, ty: OscillatorType::Square, vol: 0.3, pan: 0.0, delay: 0.0 }
    }
}

use OscillatorType::{Sawtooth, Sine, Square, Triangle};

impl Sfx {
    /// Start (or wake) the audio; browsers only allow it from a click or key press.
    pub fn wake(&mut self) {
        if self.ac.is_none() {
            if let Ok(ac) = AudioContext::new() {
                if let Ok(g) = ac.create_gain() {
                    g.gain().set_value(0.5);
                    let _ = g.connect_with_audio_node(&ac.destination());
                    self.master = Some(g);
                }
                self.ac = Some(ac);
            }
        }
        if let Some(ac) = &self.ac {
            if ac.state() == AudioContextState::Suspended {
                let _ = ac.resume();
            }
        }
    }

    fn out(&self, ac: &AudioContext, g: &GainNode, pan: f32) {
        let Some(master) = &self.master else { return };
        match ac.create_stereo_panner() {
            Ok(p) => {
                p.pan().set_value(pan.clamp(-1.0, 1.0));
                let _ = g.connect_with_audio_node(&p);
                let _ = p.connect_with_audio_node(master);
            }
            Err(_) => {
                let _ = g.connect_with_audio_node(master);
            }
        }
    }

    pub fn tone(&self, t: Tone) {
        let Some(ac) = &self.ac else { return };
        let now = ac.current_time() + t.delay;
        let (Ok(o), Ok(g)) = (ac.create_oscillator(), ac.create_gain()) else { return };
        o.set_type(t.ty);
        let _ = o.frequency().set_value_at_time(t.f0, now);
        let _ = o.frequency().exponential_ramp_to_value_at_time(t.f1.max(20.0), now + t.dur);
        let _ = g.gain().set_value_at_time(t.vol, now);
        let _ = g.gain().exponential_ramp_to_value_at_time(0.001, now + t.dur);
        let _ = o.connect_with_audio_node(&g);
        self.out(ac, &g, t.pan);
        let _ = o.start_with_when(now);
        let _ = o.stop_with_when(now + t.dur + 0.02);
    }

    pub fn noise(&mut self, dur: f64, vol: f32, f0: f32, f1: f32, pan: f32) {
        let Some(ac) = self.ac.clone() else { return };
        if self.noise.is_none() {
            let rate = ac.sample_rate();
            if let Ok(b) = ac.create_buffer(1, (rate * 1.5) as u32, rate) {
                let mut d: Vec<f32> = (0..(rate * 1.5) as usize).map(|_| (js_sys::Math::random() * 2.0 - 1.0) as f32).collect();
                let _ = b.copy_to_channel(&mut d, 0);
                self.noise = Some(b);
            }
        }
        let now = ac.current_time();
        let (Ok(src), Ok(f), Ok(g)) = (ac.create_buffer_source(), ac.create_biquad_filter(), ac.create_gain()) else { return };
        src.set_buffer(self.noise.as_ref());
        f.set_type(BiquadFilterType::Lowpass);
        let _ = f.frequency().set_value_at_time(f0, now);
        let _ = f.frequency().exponential_ramp_to_value_at_time(f1.max(20.0), now + dur);
        let _ = g.gain().set_value_at_time(vol, now);
        let _ = g.gain().exponential_ramp_to_value_at_time(0.001, now + dur);
        let _ = src.connect_with_audio_node(&f);
        let _ = f.connect_with_audio_node(&g);
        self.out(&ac, &g, pan);
        let _ = src.start_with_when(now);
        let _ = web_sys::AudioScheduledSourceNode::stop_with_when(&src, now + dur + 0.05);
    }

    pub fn pew(&self, vol: f32, pan: f32, over: bool) {
        self.tone(Tone { f0: if over { 1800.0 } else { 1400.0 }, f1: if over { 90.0 } else { 180.0 }, dur: if over { 0.25 } else { 0.13 }, ty: Sawtooth, vol, pan, ..Default::default() });
    }
    pub fn hit(&self) {
        self.tone(Tone { f0: 1900.0, f1: 1700.0, dur: 0.06, ty: Square, vol: 0.18, ..Default::default() });
        self.tone(Tone { f0: 2500.0, f1: 2400.0, dur: 0.05, ty: Square, vol: 0.12, delay: 0.05, ..Default::default() });
    }
    pub fn hurt(&self) {
        self.tone(Tone { f0: 180.0, f1: 50.0, dur: 0.25, ty: Triangle, vol: 0.5, ..Default::default() });
    }
    pub fn tagged(&self) {
        for (i, f) in [600.0, 450.0, 300.0, 160.0].iter().enumerate() {
            self.tone(Tone { f0: *f, f1: f * 0.8, dur: 0.16, ty: Square, vol: 0.2, delay: i as f64 * 0.11, ..Default::default() });
        }
    }
    pub fn scored(&self) {
        for (i, f) in [520.0, 660.0, 880.0, 1170.0].iter().enumerate() {
            self.tone(Tone { f0: *f, f1: f * 1.02, dur: 0.12, ty: Square, vol: 0.16, delay: i as f64 * 0.07, ..Default::default() });
        }
    }
    pub fn pick(&self) {
        self.tone(Tone { f0: 300.0, f1: 1600.0, dur: 0.35, ty: Sine, vol: 0.35, ..Default::default() });
    }
    pub fn dry(&self) {
        self.tone(Tone { f0: 160.0, f1: 120.0, dur: 0.06, ty: Square, vol: 0.12, ..Default::default() });
    }
    pub fn overheat(&self) {
        self.tone(Tone { f0: 900.0, f1: 120.0, dur: 0.5, ty: Sawtooth, vol: 0.2, ..Default::default() });
        self.tone(Tone { f0: 3000.0, f1: 2000.0, dur: 0.4, ty: Sine, vol: 0.05, delay: 0.1, ..Default::default() });
    }
    pub fn deflect(&self) {
        self.tone(Tone { f0: 3000.0, f1: 1200.0, dur: 0.12, ty: Triangle, vol: 0.15, ..Default::default() });
    }
    pub fn jump(&self) {
        self.tone(Tone { f0: 220.0, f1: 440.0, dur: 0.1, ty: Sine, vol: 0.1, ..Default::default() });
    }
    pub fn launch(&mut self, vol: f32, pan: f32) {
        self.noise(0.7, vol, 600.0, 3000.0, pan);
        self.tone(Tone { f0: 200.0, f1: 900.0, dur: 0.5, ty: Sawtooth, vol: vol * 0.25, pan, ..Default::default() });
    }
    pub fn boom(&mut self, vol: f32, pan: f32) {
        self.noise(1.2, vol, 1400.0, 60.0, pan);
        self.tone(Tone { f0: 110.0, f1: 30.0, dur: 0.8, ty: Sine, vol: vol * 0.8, pan, ..Default::default() });
    }
    pub fn fizzle(&mut self) {
        self.noise(0.4, 0.2, 3000.0, 400.0, 0.0);
    }
    pub fn lock(&self) {
        self.tone(Tone { f0: 1600.0, f1: 1600.0, dur: 0.05, ty: Square, vol: 0.06, ..Default::default() });
    }
    pub fn drone_out(&self) {
        self.tone(Tone { f0: 300.0, f1: 1200.0, dur: 0.35, ty: Sine, vol: 0.25, ..Default::default() });
        self.tone(Tone { f0: 120.0, f1: 180.0, dur: 0.6, ty: Sawtooth, vol: 0.08, ..Default::default() });
    }
    pub fn drone_hum(&self) {
        let f = 150.0 + js_sys::Math::random() as f32 * 30.0;
        self.tone(Tone { f0: f, f1: 140.0, dur: 0.12, ty: Sawtooth, vol: 0.03, ..Default::default() });
    }
}
