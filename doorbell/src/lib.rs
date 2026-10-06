// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! Doorbell widget — the picture from a video doorbell, and an overlay plus a
//! blinking light strip when someone rings.
//!
//! Built on the SDK's Image widget (the picture pipeline lives in
//! `remote-image`). The doorbell service on the Deck (`deck/doorbell`) does
//! the listening and the chime; this widget reads its status and picture
//! from the Deck's own web server, so it needs no setup.

mod manifest_params;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use super::manifest_params::{self, Sizing};
    use std::cell::{Cell, RefCell};

    #[expect(
        clippy::wildcard_imports,
        reason = "widget render uses many SDK exports"
    )]
    use bmc_wasm_sdk::*;
    use remote_image::machine::{self, Action, Badge, Event, View};
    use remote_image::{Fit, picture, render};

    /// Menu clears itself this long after opening, untouched.
    const MENU_AUTO_DISMISS_MS: u32 = 10_000;

    const CONFIGURE_URL: &str = "Set a picture URL";

    // ── Doorbell ──────────────────────────────────────────────────────────
    // The doorbell service puts this scene on screen the moment the VTO
    // rings; awake, the widget reads the service's status and shows who rang.
    // The chime comes from the service too (madplay): on this firmware the
    // wasm host is built without audio, so a widget's `audio_play` is silent.
    // Asleep it polls nothing, like the picture itself.
    const RING_POLL_MS: u32 = 1_500;
    /// How long after a ring the overlay stays.
    const RING_FRESH_SECS: i64 = 120;
    const GOLD: Color = Color::from_hex(0xFF_C8_3D);
    /// What the strip blinks by default: a clear yellow (the LEDs render
    /// the screen's gold too orange).
    const BELL_YELLOW: Color = Color::from_hex(0xFF_C8_00);
    /// One blink: fast enough to read as a doorbell, not as the ambient breath.
    const BLINK_MS: u32 = 600;
    const BLINK_FOR_MS: u32 = 30_000;

    thread_local! {
        static VIEW: RefCell<View> = const { RefCell::new(View::Loading { decode: None }) };
        static POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
        // Menu auto-dismiss countdown (ms); 0 = closed.
        static MENU_MS: Cell<u32> = const { Cell::new(0) };
        // First render restores from cache (init() has no renderer scope).
        static INITIAL_RESTORE: Cell<bool> = const { Cell::new(false) };
        /// Off-scene, between `on_sleep` and `on_wake`. The host keeps
        /// delivering params and fetch replies there, and no SDK call answers
        /// it, so the edge is tracked by hand. It starts set: every slot is
        /// born dormant and `on_wake` always precedes the first frame, so a
        /// widget built for a scene nobody opens must not fetch a picture.
        static DORMANT: Cell<bool> = const { Cell::new(true) };
        static RING_POLL: Cell<Option<PollHandle>> = const { Cell::new(None) };
        /// Unix time of the last ring the service reported, and its "08:15".
        static RING_AT: Cell<i64> = const { Cell::new(0) };
        static RING_TIME: RefCell<String> = const { RefCell::new(String::new()) };
        static RING_NAME: RefCell<String> = const { RefCell::new(String::new()) };
        /// The ring the strip already lit up for — once per press.
        static LIT_AT: Cell<i64> = const { Cell::new(0) };
        /// The doorbell's own settings, carried in its status: blink the
        /// strip or not, in which colour, and the overlay's language.
        static RING_LED: Cell<Option<Color>> = const { Cell::new(Some(BELL_YELLOW)) };
        static RING_EN: Cell<bool> = const { Cell::new(false) };
        /// The service allows opening the door (its `unlock` switch).
        static UNLOCK_ON: Cell<bool> = const { Cell::new(false) };
        static LOCK: Cell<Lock> = const { Cell::new(Lock::Idle) };
    }

    fn ring_url() -> Option<String> {
        Some(manifest_params::Params::current().ring_url).filter(|u| !u.trim().is_empty())
    }

    fn build_ring(_h: PollHandle) -> Option<FetchSpec> {
        ring_url().map(|u| FetchSpec::get(u).timeout(core::time::Duration::from_secs(3)))
    }

    fn on_ring(_h: PollHandle, r: &FetchResponse) {
        if !r.ok() {
            return;
        }
        let j = r.json();
        let at = j.i64("/last_ring/at").unwrap_or(0);
        RING_AT.set(at);
        RING_TIME.with(|t| *t.borrow_mut() = j.str("/last_ring/time").unwrap_or_default());
        RING_NAME.with(|t| *t.borrow_mut() = j.str("/last_ring/name").unwrap_or_default());
        RING_EN.set(j.str("/lang").as_deref() == Some("en"));
        UNLOCK_ON.set(j.bool("/unlock").unwrap_or(false));
        let led_on = j.bool("/led/on").unwrap_or(true);
        let color = j.str("/led/color").and_then(|c| parse_hex(&c)).map_or(BELL_YELLOW, |(r, g, b)| Color::from_rgb(r, g, b));
        RING_LED.set(led_on.then_some(color));
        if ringing() && LIT_AT.get() != at {
            LIT_AT.set(at);
            if let Some(c) = RING_LED.get() {
                // Blink the strip while someone waits at the door.
                led::set_effect(LedEffect::Breathe, c, BLINK_MS, Some(BLINK_FOR_MS));
            }
        }
        request_frame();
    }

    /// "RRGGBB" (optionally with '#') into RGB.
    fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        #[expect(clippy::cast_possible_truncation, reason = "masked to one byte each")]
        Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    // ── Opening the door ─────────────────────────────────────────────────
    // Only when the service's `unlock` switch is on. Two taps: the first arms
    // the button for a few seconds, the second opens — a passing swipe or a
    // curious cat should never open the front door.

    const KEY_UNLOCK: &str = "unlock";
    const UNLOCK_ARM_S: i64 = 4;
    const UNLOCK_SHOW_S: i64 = 5;
    const GREEN: Color = Color::from_hex(0x30_D1_58);
    const RED: Color = Color::from_hex(0xFF_45_3A);

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Lock {
        Idle,
        Armed(i64),
        Busy,
        Done(i64),
        Failed(i64),
    }

    /// The button's state now: an arming or a result that has run its time is idle again.
    fn lock_now(now: i64) -> Lock {
        match LOCK.get() {
            Lock::Armed(t) if now - t >= UNLOCK_ARM_S => Lock::Idle,
            Lock::Done(t) | Lock::Failed(t) if now - t >= UNLOCK_SHOW_S => Lock::Idle,
            l => l,
        }
    }

    /// The service's unlock CGI, next to its status CGI.
    fn unlock_url() -> Option<String> {
        ring_url().and_then(|u| u.rfind('/').map(|i| fmt!("{}/doorbell-unlock", &u[..i])))
    }

    fn on_unlock(r: &FetchResponse) {
        let ok = r.ok() && r.json().bool("/ok").unwrap_or(false);
        let now = SystemTime::now().unix_secs;
        LOCK.set(if ok { Lock::Done(now) } else { Lock::Failed(now) });
        request_frame();
    }

    fn unlock_tapped() {
        let now = SystemTime::now().unix_secs;
        match lock_now(now) {
            Lock::Armed(_) => {
                if let Some(url) = unlock_url() {
                    LOCK.set(Lock::Busy);
                    let _ = FetchRequest::post(&url).timeout(core::time::Duration::from_secs(12)).send(on_unlock);
                }
            }
            Lock::Busy => {}
            _ => LOCK.set(Lock::Armed(now)),
        }
        request_frame();
    }

    /// An open padlock in an `s`-sized box at (`ox`, `oy`).
    fn padlock(s: f32, ox: f32, oy: f32, color: Color) -> Vec<Draw> {
        let p = |x: f32, y: f32| (ox + s * x, oy + s * y);
        let mut shackle = vec![p(0.65, 0.47)];
        // Over the top from right to left; the left leg stops short of the body — open.
        shackle.extend((0..=12).map(|i| {
            #[expect(clippy::cast_precision_loss, reason = "twelve steps")]
            let a = core::f32::consts::PI * (i as f32 / 12.0);
            p(0.50 + 0.15 * a.cos(), 0.30 - 0.15 * a.sin())
        }));
        shackle.push(p(0.35, 0.36));
        let ink = Color::from_rgba(0, 0, 0, 0x70);
        vec![
            Draw::path(shackle, s * 0.075, color, false, Interpolation::Linear),
            Draw::fill_path(round_rect(ox + s * 0.24, oy + s * 0.45, s * 0.52, s * 0.38, s * 0.07), color, true),
            Draw::circle(ox + s * 0.50, oy + s * 0.60, s * 0.045, ink),
            Draw::rect(ox + s * 0.485, oy + s * 0.62, s * 0.03, s * 0.10, ink),
        ]
    }

    fn round_rect(left: f32, top: f32, width: f32, height: f32, radius: f32) -> Vec<(f32, f32)> {
        let corners = [
            (left + width - radius, top + radius, -90.0_f32),
            (left + width - radius, top + height - radius, 0.0),
            (left + radius, top + height - radius, 90.0),
            (left + radius, top + radius, 180.0),
        ];
        let mut pts = Vec::with_capacity(28);
        for (cx, cy, start) in corners {
            for step in 0..7u8 {
                let angle = (start + f32::from(step) * 15.0).to_radians();
                pts.push((cx + radius * angle.cos(), cy + radius * angle.sin()));
            }
        }
        pts
    }

    /// The round green button, bottom right, with its label underneath.
    fn unlock_button(base: Node, height: f32) -> Node {
        let now = SystemTime::now().unix_secs;
        let en = RING_EN.get();
        let state = lock_now(now);
        let disc_d = if height >= 300.0 { 76.0 } else { 52.0 };
        let (fill, label) = match state {
            Lock::Idle => (GREEN, if en { "Unlock" } else { "Ontgrendelen" }),
            Lock::Armed(_) => (GREEN, if en { "Tap again" } else { "Nogmaals tikken" }),
            Lock::Busy => (GREEN, if en { "Opening…" } else { "Openen…" }),
            Lock::Done(_) => (GREEN, if en { "Unlocked" } else { "Geopend" }),
            Lock::Failed(_) => (RED, if en { "Failed" } else { "Mislukt" }),
        };
        let margin = 6.0; // room around the disc for the shadow and the "armed" ring
        let center = margin + disc_d / 2.0;
        let mut draws = vec![Draw::circle(center, center + 2.0, disc_d / 2.0, Color::from_rgba(0, 0, 0, 0x60))];
        let disc = Draw::circle(center, center, disc_d / 2.0, fill.with_alpha(if state == Lock::Idle { 0.85 } else { 1.0 }));
        draws.push(if matches!(state, Lock::Armed(_)) {
            // A pulse while it waits for the second tap.
            disc.animate(AnimProperty::Alpha, 1.0, 0.55, 420, Easing::EaseInOut, LoopMode::PingPong)
        } else {
            disc
        });
        if matches!(state, Lock::Armed(_)) {
            draws.push(Draw::arc(center, center, disc_d / 2.0 + 4.0, 0.0, core::f32::consts::TAU, 3.0, ArcFill::Solid(WHITE), ArcSegments::Continuous, ArcCap::Round));
        }
        let glyph = disc_d * 0.62;
        draws.extend(padlock(glyph, center - glyph / 2.0, center - glyph / 2.0, WHITE));
        let small = height < 300.0;
        let button = col(
            props!(inset_right: 22.0, inset_bottom: 18.0, gap: 6.0, cross_align: CrossAlign::Center),
            [
                touchable(KEY_UNLOCK, props!(width: disc_d + 2.0 * margin, height: disc_d + 2.0 * margin), draws),
                row(
                    props!(padding: 5.0, background: Color::from_rgba(0x0E, 0x0C, 0x0A, 0xB0), border_radius: 10.0),
                    [text(label, style!(size: if small { 11 } else { 14 }, weight: FontWeight::SEMIBOLD, color: WHITE))],
                ),
            ],
        );
        let mut base = base;
        if let Node::Column(_, children) | Node::Row(_, children) = &mut base {
            children.push(button);
        }
        if state != Lock::Idle {
            request_frame_after(1_000);
        }
        base
    }

    fn ringing() -> bool {
        let at = RING_AT.get();
        at > 0 && SystemTime::now().unix_secs - at < RING_FRESH_SECS
    }

    fn with_ring_poll(f: impl FnOnce(PollHandle)) {
        if let Some(h) = RING_POLL.get() {
            f(h);
        }
    }

    /// Gold frame breathing around the picture, and a pill with a swinging bell.
    fn ring_overlay(base: Node, w: f32, h: f32) -> Node {
        let edge = (h * 0.012).max(3.0);
        let frame = canvas(
            props!(inset_top: 0.0, inset_left: 0.0, width: w, height: h),
            [
                Draw::rect(0.0, 0.0, w, edge, GOLD),
                Draw::rect(0.0, h - edge, w, edge, GOLD),
                Draw::rect(0.0, 0.0, edge, h, GOLD),
                Draw::rect(w - edge, 0.0, edge, h, GOLD),
            ]
            .map(|d| d.animate(AnimProperty::Alpha, 1.0, 0.25, 900, Easing::EaseInOut, LoopMode::PingPong)),
        );
        let big = h >= 300.0;
        let (title_px, sub_px, icon) = if big { (26, 16, 34.0) } else { (16, 12, 22.0) };
        let bell = {
            let s = icon;
            let body = vec![
                (s * 0.50, s * 0.10),
                (s * 0.72, s * 0.22),
                (s * 0.78, s * 0.52),
                (s * 0.90, s * 0.74),
                (s * 0.10, s * 0.74),
                (s * 0.22, s * 0.52),
                (s * 0.28, s * 0.22),
            ];
            canvas(
                props!(width: s, height: s),
                [
                    Draw::fill_path(body, GOLD, true),
                    Draw::circle(s * 0.50, s * 0.84, s * 0.09, GOLD),
                    Draw::circle(s * 0.50, s * 0.08, s * 0.05, GOLD),
                ]
                .map(|d| d.animate(AnimProperty::Rotate, -0.35, 0.35, 260, Easing::EaseInOut, LoopMode::PingPong)),
            )
        };
        let time = RING_TIME.with(|t| t.borrow().clone());
        let name = RING_NAME.with(|t| t.borrow().clone());
        let sub = match (time.is_empty(), name.is_empty()) {
            (false, false) => fmt!("{} · {}", time, name),
            (false, true) => time,
            (true, false) => name,
            (true, true) => String::new(),
        };
        let pill = row(
            props!(
                inset_top: edge + 14.0,
                inset_left: edge + 14.0,
                gap: 12.0,
                padding: if big { 14.0 } else { 9.0 },
                background: Color::from_rgba(0x0E, 0x0C, 0x0A, 0xD8),
                border_radius: 22.0,
                border_width: 1.0,
                border_color: Color::from_rgba(0xFF, 0xC8, 0x3D, 0x70),
                cross_align: CrossAlign::Center,
            ),
            [
                bell,
                col(
                    props!(gap: 2.0),
                    [
                        text(if RING_EN.get() { "Someone's at the door" } else { "Er wordt aangebeld" }, style!(size: title_px, weight: FontWeight::BOLD, color: WHITE)),
                        text(sub, style!(size: sub_px, color: Color::from_hex(0xC9_BF_B3))),
                    ],
                ),
            ],
        );
        // Absolute children ride on the picture's own column, like the SDK's status overlays.
        let mut base = base;
        if let Node::Column(_, children) | Node::Row(_, children) = &mut base {
            children.push(frame);
            children.push(pill);
        }
        base
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        let handle = register_poll(
            build_request,
            on_image,
            PollConfig {
                interval_ms: Some(refresh_interval_ms()),
                enabled: false, // first render restores + schedules
                ..Default::default()
            },
        );
        POLL.with(|p| p.set(Some(handle)));
        INITIAL_RESTORE.with(|f| f.set(true));
        let ring = register_poll(
            build_ring,
            on_ring,
            PollConfig { interval_ms: Some(RING_POLL_MS), debounce_ms: 0, retry_ms: 3_000, enabled: false, ..Default::default() },
        );
        RING_POLL.set(Some(ring));
    }

    /// Fold an event into the view and run its side effects.
    fn dispatch(event: Event) {
        let cur = VIEW.with(|v| v.replace(View::Loading { decode: None }));
        let (next, actions) = machine::step(cur, event);
        VIEW.with(|v| *v.borrow_mut() = next);
        let dormant = DORMANT.with(Cell::get);
        for action in actions {
            if dormant && action.starts_fetch() {
                continue;
            }
            match action {
                Action::EnablePollAfter(ms) => with_poll(|h| h.enable_after(ms)),
                Action::ResumePoll => with_poll(|h| {
                    h.set_enabled(true);
                    h.invalidate();
                }),
                Action::DisablePoll => with_poll(|h| h.set_enabled(false)),
                Action::Retry => with_poll(PollHandle::retry),
                Action::DeferPoll => with_poll(|h| h.retry_after(refresh_interval_ms())),
                Action::MarkStale => with_poll(PollHandle::mark_stale),
                Action::SeedAnchor(secs) => with_poll(|h| h.restore_anchor(secs)),
                Action::RequestFrame => request_frame(),
            }
        }
    }

    fn with_poll(f: impl FnOnce(PollHandle)) {
        POLL.with(|p| {
            if let Some(handle) = p.get() {
                f(handle);
            }
        });
    }

    // Zero guard only, deliberately not the manifest's `min`. Staleness fires
    // at `interval * stale_factor`, so clamping a stored interval up here
    // would move that threshold too — quietly weakening the configured freshness.
    fn refresh_interval_ms() -> u32 {
        let secs = manifest_params::Params::current().refresh_seconds.max(1);
        u32::try_from(secs).unwrap_or(u32::MAX).saturating_mul(1000)
    }

    fn fit() -> Fit {
        match manifest_params::Params::current().sizing {
            Sizing::Contain => Fit::Contain,
            Sizing::Cover => Fit::Cover,
        }
    }

    // {{width}}/{{height}} expand to the viewport pixels — 1:1 with the
    // released Slint widget so existing server URLs carry over unchanged.
    fn expanded_url() -> Option<String> {
        let size = widget_size();
        machine::expand_url(
            &manifest_params::Params::current().url,
            size.width,
            size.height,
        )
    }

    // Cache identity: expanded URL + fit, so a URL/viewport/sizing change is a distinct blob.
    fn cache_identity() -> Option<String> {
        let mut id = expanded_url()?;
        id.push('\u{1f}');
        id.push_str(fit().identity_token());
        Some(id)
    }

    fn build_request(_handle: PollHandle) -> Option<FetchSpec> {
        expanded_url().map(|url| FetchSpec::get(url).host_body())
    }

    /// Bring back whichever picture is on flash. Render scope only.
    fn restore_from_cache() -> Event {
        picture::restore(&cache_identity().unwrap_or_default(), refresh_interval_ms())
    }

    fn on_image(_handle: PollHandle, response: &FetchResponse) {
        dispatch(picture::classify_body(
            response,
            widget_size(),
            fit(),
            &cache_identity().unwrap_or_default(),
            VIEW.with(|v| v.borrow().decode()),
            on_decoded,
        ));
    }

    fn on_decoded(job: ImageJobId, bitmap: Option<BitmapId>) {
        dispatch(match bitmap {
            Some(bitmap) => Event::Decoded { job, bitmap },
            None => Event::DecodeFailed { job },
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        let prev = manifest_params::Params::previous();
        let cur = manifest_params::Params::current();
        // Retarget the poll live on a refresh-period change; invalidate to apply
        // now. Both are safe off-screen: the interval is only stored, and
        // `invalidate` skips a poll `on_sleep` disabled.
        if prev
            .as_ref()
            .is_some_and(|p| p.refresh_seconds != cur.refresh_seconds)
        {
            with_poll(|handle| {
                handle.set_interval(refresh_interval_ms());
                handle.invalidate();
            });
        }
        if prev
            .as_ref()
            .is_none_or(|p| p.url != cur.url || p.sizing != cur.sizing)
        {
            dispatch(Event::TargetChanged);
        } else {
            request_frame();
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_system_update() {
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_touch() {
        // The host only delivers touch to on_touch exporters; wake a frame to read it.
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_sleep() {
        dispatch(Event::Sleep);
        DORMANT.with(|d| d.set(true));
        with_ring_poll(|h| h.set_enabled(false));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_wake() {
        DORMANT.with(|d| d.set(false));
        INITIAL_RESTORE.with(|f| f.set(false)); // wake subsumes the cold-start restore
        if let Some(job) = VIEW.with(|v| picture::abandoned_decode(&v.borrow())) {
            dispatch(Event::DecodeAbandoned { job });
        }
        dispatch(restore_from_cache());
        if ring_url().is_some() {
            with_ring_poll(|h| {
                h.set_enabled(true);
                h.invalidate();
            });
        }
    }

    fn menu_open() -> bool {
        MENU_MS.with(Cell::get) > 0
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn render(delta_ms: u32) {
        // First render restores from cache (init() has no renderer scope).
        if INITIAL_RESTORE.with(Cell::get) {
            INITIAL_RESTORE.with(|f| f.set(false));
            dispatch(restore_from_cache());
        }

        if menu_open() {
            let remaining = MENU_MS.with(Cell::get).saturating_sub(delta_ms);
            MENU_MS.with(|m| m.set(remaining));
        }

        let size = widget_size();
        let params = manifest_params::Params::current();
        let base = if params.url.trim().is_empty() {
            render::message_view(CONFIGURE_URL, size)
        } else {
            VIEW.with(|v| match &*v.borrow() {
                View::Shown {
                    bitmap,
                    aspect,
                    badge,
                    ..
                } => {
                    let view = render::image_view(*bitmap, *aspect, size, fit());
                    match badge {
                        Badge::Updating => {
                            with_overlay(view, render::updating_pill(), widget_viewport().shape)
                        }
                        // is_stale adds the grace window, so the 10s retry heals a blip first.
                        Badge::Stale => match POLL
                            .with(Cell::get)
                            .filter(|handle| handle.is_stale())
                            .and_then(PollHandle::last_success_time)
                        {
                            Some(anchor) => {
                                with_stale_overlay(view, anchor, widget_viewport().shape)
                            }
                            None => view,
                        },
                        // A broken payload states its specific reason at once.
                        Badge::Error(kind) => with_error_overlay(
                            view,
                            render::error_message(*kind),
                            widget_viewport().shape,
                        ),
                        Badge::Fresh => view,
                    }
                }
                View::Loading { .. } => render::message_view(render::LOADING, size),
                View::Failed(kind) => render::message_view(render::error_message(*kind), size),
            })
        };

        #[expect(clippy::cast_precision_loss, reason = "viewport sizes are small integers")]
        let base = if ringing() { ring_overlay(base, size.width as f32, size.height as f32) } else { base };
        if ringing() {
            // Drop the overlay on time even if nothing else changes.
            request_frame_after(5_000);
        }

        #[expect(clippy::cast_precision_loss, reason = "viewport sizes are small integers")]
        let base = if UNLOCK_ON.get() && unlock_url().is_some() { unlock_button(base, size.height as f32) } else { base };

        let open = menu_open();
        let result = render_ui(
            size.width,
            size.height,
            // No whole-face tap catcher (it costs the Deck ~13 % CPU while shown);
            // the reload menu stays reachable only if it was already open.
            if open { render::with_interaction(base, open) } else { base },
        );

        if result.clicks.contains_key(KEY_UNLOCK) && UNLOCK_ON.get() {
            unlock_tapped();
        }

        // Route taps: a button when the menu is open, otherwise open/dismiss it.
        let changed = if open {
            if result.clicks.contains_key(render::KEY_RELOAD) {
                dispatch(Event::Reload);
                MENU_MS.with(|m| m.set(0));
                true
            } else if result.clicks.contains_key(render::KEY_CLOSE)
                || result.clicks.contains_key(render::KEY_TAP)
            {
                MENU_MS.with(|m| m.set(0));
                true
            } else {
                false
            }
        } else if result.clicks.contains_key(render::KEY_TAP) {
            MENU_MS.with(|m| m.set(MENU_AUTO_DISMISS_MS));
            true
        } else {
            false
        };

        // Re-render on a change; else hold one frame at the dismiss deadline.
        if changed {
            request_frame();
        } else if menu_open() {
            request_frame_after(MENU_MS.with(Cell::get));
        }
    }
}
