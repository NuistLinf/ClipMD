//! 动画：弹簧和帧时钟。
//!
//! 弹簧比固定时长的缓动更自然：中途改目标时保留速度，不会突然顿一下（SwiftUI 默认也是弹簧）。
//! 帧时钟用一个后台线程调 DwmFlush（等到下一次屏幕合成），再给需要动画的窗口发消息，
//! 所以帧和屏幕刷新对齐；没有动画时线程睡着，不占 CPU。

use std::sync::{Condvar, Mutex, Once};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// 窗口收到这个消息就推进一帧动画
pub const WM_FRAME: u32 = WM_APP + 1;

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    /// 回弹的快慢：响应时间（秒），越小越快
    response: f32,
    /// 阻尼比：1 不回弹，小于 1 会稍微冲过头
    damping: f32,
}

impl Spring {
    pub fn new(value: f32) -> Spring {
        Spring { value, velocity: 0.0, target: value, response: 0.32, damping: 0.86 }
    }

    pub fn with(value: f32, response: f32, damping: f32) -> Spring {
        Spring { value, velocity: 0.0, target: value, response, damping }
    }

    pub fn set(&mut self, target: f32) {
        self.target = target;
    }

    /// 直接跳到某个值，不做动画
    pub fn snap(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    pub fn is_moving(&self) -> bool {
        (self.value - self.target).abs() > 0.001 || self.velocity.abs() > 0.01
    }

    /// 推进 `dt` 秒，返回是否还在动
    pub fn step(&mut self, dt: f32) -> bool {
        if !self.is_moving() {
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }
        let omega = std::f32::consts::TAU / self.response.max(0.01);
        let stiffness = omega * omega;
        let damping = 2.0 * self.damping * omega;
        // 分成小步积分，帧率低时也稳定
        let mut remaining = dt.min(0.1);
        while remaining > 0.0 {
            let h = remaining.min(1.0 / 240.0);
            let force = -stiffness * (self.value - self.target) - damping * self.velocity;
            self.velocity += force * h;
            self.value += self.velocity * h;
            remaining -= h;
        }
        if !self.is_moving() {
            self.value = self.target;
            self.velocity = 0.0;
        }
        true
    }
}

/// 等着下一帧的窗口
static WANTED: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static SIGNAL: Condvar = Condvar::new();
static STARTED: Once = Once::new();

fn run_pump() {
    loop {
        {
            let mut wanted = WANTED.lock().unwrap();
            while wanted.is_empty() {
                wanted = SIGNAL.wait(wanted).unwrap();
            }
        }
        // 等下一次屏幕合成；DWM 没开（极少见）时退回固定间隔
        if unsafe { DwmFlush() }.is_err() {
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        let targets: Vec<isize> = std::mem::take(&mut *WANTED.lock().unwrap());
        for hwnd in targets {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_FRAME, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// 请求在下一帧给 `hwnd` 发 WM_FRAME
pub fn request_frame(hwnd: HWND) {
    STARTED.call_once(|| {
        std::thread::Builder::new().name("frame-clock".into()).spawn(run_pump).expect("创建动画线程失败");
    });
    let mut wanted = WANTED.lock().unwrap();
    let id = hwnd.0 as isize;
    if !wanted.contains(&id) {
        wanted.push(id);
    }
    SIGNAL.notify_one();
}

/// 帧之间的时间，限制在合理范围，窗口刚开始动画时按一帧算
pub struct FrameTimer {
    last: Option<f64>,
}

impl FrameTimer {
    pub const fn new() -> FrameTimer {
        FrameTimer { last: None }
    }

    pub fn tick(&mut self) -> f32 {
        let now = crate::util::now_seconds();
        let dt = match self.last {
            Some(last) => (now - last) as f32,
            None => 1.0 / 60.0,
        };
        self.last = Some(now);
        dt.clamp(1.0 / 240.0, 1.0 / 20.0)
    }

    /// 动画停了，下次开始时重新计时
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// 缓动曲线（少数固定时长的过渡用，比如提示框淡出）
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}
