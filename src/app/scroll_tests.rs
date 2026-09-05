//! Exercise scrolling through the complete application frame, including its filter.
use std::time::{Duration, Instant};

use egui::{Event, Modifiers, MouseWheelUnit, Pos2, Rect, TouchPhase, Vec2, vec2};

use crate::{
    app::{App, AppOptions},
    backend::Waker,
    demo,
    paths::AppDirs,
    settings::Settings,
};

struct Home {
    app: App,
    ctx: egui::Context,
    frame: u32,
    started: Instant,
    title: String,
    root: std::path::PathBuf,
}

impl Home {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("fastpotify-scroll-{name}-{}", std::process::id()));
        let ctx = egui::Context::default();
        let mut app = App::new(
            &Waker::default(),
            AppDirs {
                config: root.join("config"),
                state: root.join("state"),
                cache: root.join("cache"),
            },
            Settings::default(),
            AppOptions {
                media_controls: false,
                tray: false,
            },
        );
        app.attach(&ctx);
        demo::populate(&mut app);
        let title = app.home.recently_played.get().unwrap()[2]
            .track
            .name
            .clone();
        let mut home = Self {
            app,
            ctx,
            frame: 0,
            started: Instant::now(),
            title,
            root,
        };
        for _ in 0..5 {
            home.draw(vec![]);
        }
        home
    }

    fn draw(&mut self, events: Vec<Event>) -> Pos2 {
        self.frame += 1;
        let time = f64::from(self.frame) / 60.0;
        let now = self.started + Duration::from_secs_f64(time);
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1240.0, 1100.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| self.app.frame_ui_at(ui, now),
        );
        output.textures_delta.clear();
        fn position(shape: &egui::Shape, title: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == title => Some(text.pos),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| position(shape, title)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| position(&shape.shape, &self.title))
            .expect("the sampled card is visible on the Home shelf")
    }

    fn scroll(&mut self, unit: MouseWheelUnit, delta: Vec2, modifiers: Modifiers) -> (Pos2, Pos2) {
        let before = self.draw(vec![]);
        let pointer = before + vec2(40.0, -70.0);
        for _ in 0..2 {
            self.draw(vec![Event::PointerMoved(pointer)]);
        }
        self.draw(vec![Event::MouseWheel {
            unit,
            delta,
            phase: TouchPhase::Move,
            modifiers,
        }]);
        // egui smooths wheel ticks over several frames. Point events apply
        // immediately, so keep successive trackpad events in one gesture.
        if unit != MouseWheelUnit::Point {
            for _ in 0..10 {
                self.draw(vec![]);
            }
        }
        (before, self.draw(vec![]))
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        self.app.backend.shutdown();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn shift_wheel_scrolls_the_home_shelf() {
    let mut home = Home::new("shift");
    let (before, after) = home.scroll(MouseWheelUnit::Line, vec2(0.0, -1.0), Modifiers::SHIFT);
    let mut horizontal = Home::new("horizontal");
    let (control_before, control_after) =
        horizontal.scroll(MouseWheelUnit::Line, vec2(-1.0, 0.0), Modifiers::NONE);
    assert!(
        after.x < before.x - 1.0,
        "Shift+wheel must move the cards sideways: {before:?} -> {after:?}"
    );
    assert!(
        (after.y - before.y).abs() < 1.0,
        "Shift+wheel must not move the page vertically"
    );
    assert!(
        ((before.x - after.x) - (control_before.x - control_after.x)).abs() < 1.0,
        "Shift+wheel must preserve the same movement as a horizontal wheel"
    );
}

#[test]
fn wheel_direction_changes_as_shift_is_pressed_and_released() {
    let mut home = Home::new("modifiers");
    for modifiers in [Modifiers::NONE, Modifiers::SHIFT, Modifiers::NONE] {
        let (before, after) = home.scroll(MouseWheelUnit::Line, vec2(0.0, -0.5), modifiers);
        if modifiers.shift {
            assert!(
                after.x < before.x - 1.0,
                "pressing Shift must override the previous vertical gesture"
            );
            assert!((after.y - before.y).abs() < 1.0);
        } else {
            assert!(
                after.y < before.y - 1.0,
                "releasing Shift must allow the page to scroll vertically"
            );
            assert!((after.x - before.x).abs() < 1.0);
        }
    }
}

#[test]
fn horizontal_wheel_can_follow_a_vertical_trackpad_gesture() {
    let mut home = Home::new("wheel-after-trackpad");
    home.scroll(MouseWheelUnit::Point, vec2(0.1, -3.0), Modifiers::NONE);
    let (before, after) = home.scroll(MouseWheelUnit::Line, vec2(-0.5, 0.0), Modifiers::NONE);
    assert!(
        after.x < before.x - 1.0,
        "a wheel must not inherit the trackpad's vertical lock"
    );
    assert!((after.y - before.y).abs() < 1.0);
}

#[test]
fn trackpad_gestures_still_reject_cross_axis_drift() {
    let mut home = Home::new("trackpad");
    let (before, after) = home.scroll(MouseWheelUnit::Point, vec2(-3.0, 0.1), Modifiers::NONE);
    assert!(after.x < before.x - 1.0);
    assert!((after.y - before.y).abs() < 1.0);
    let (before, after) = home.scroll(MouseWheelUnit::Point, vec2(-0.2, -3.0), Modifiers::NONE);
    assert!(
        (after.y - before.y).abs() < 1.0,
        "a horizontal trackpad gesture must keep its axis"
    );
}
