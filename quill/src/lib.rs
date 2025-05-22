pub mod prelude {
    pub use crate::blue;
    pub use crate::bold;
    pub use crate::color;
    pub use crate::cyan;
    pub use crate::green;
    pub use crate::magenta;
    pub use crate::red;
    pub use crate::underline;
    pub use crate::white;
    pub use crate::yellow;
}

static USE_ANSI: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn init(ansi: bool) {
    USE_ANSI.store(ansi, std::sync::atomic::Ordering::SeqCst);
}

pub fn should_use_ansi() -> bool {
    USE_ANSI.load(std::sync::atomic::Ordering::SeqCst)
}

// underline
#[macro_export]
macro_rules! underline {
    ($fmt:expr) => {
        if $crate::should_use_ansi() {
            format!("\x1b[4m{}\x1b[0m", $fmt)
        } else {
            format!("{}", $fmt)
        }
    };
    ($fmt:expr, $($arg:tt)*) => {
        if $crate::should_use_ansi() {
            format!("\x1b[4m{}\x1b[0m", format!($fmt, $($arg)*))
        } else {
            format!($fmt, $($arg)*)
        }
    };
}

// bold
#[macro_export]
macro_rules! bold {
    ($fmt:expr) => {
        if $crate::should_use_ansi() {
            format!("\x1b[1m{}\x1b[0m", $fmt)
        } else {
            format!("{}", $fmt)
        }
    };
    ($fmt:expr, $($arg:tt)*) => {
        if $crate::should_use_ansi() {
            format!("\x1b[1m{}\x1b[0m", format!($fmt, $($arg)*))
        } else {
            format!($fmt, $($arg)*)
        }
    };
}

pub enum Color {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

impl Color {
    pub fn to_ansi(&self) -> &'static str {
        match self {
            Color::Red => "\x1b[31m",
            Color::Green => "\x1b[32m",
            Color::Yellow => "\x1b[33m",
            Color::Blue => "\x1b[34m",
            Color::Magenta => "\x1b[35m",
            Color::Cyan => "\x1b[36m",
            Color::White => "\x1b[37m",
        }
    }
}

// color
#[macro_export]
macro_rules! color {
    ($color:expr, $fmt:expr) => {
        if $crate::should_use_ansi() {
            format!("{}{}\x1b[0m", $color.to_ansi(), $fmt)
        } else {
            format!("{}", $fmt)
        }
    };
    ($color:expr, $fmt:expr, $($arg:tt)*) => {
        if $crate::should_use_ansi() {
            format!("{}{}\x1b[0m", $color.to_ansi(), format!($fmt, $($arg)*))
        } else {
            format!($fmt, $($arg)*)
        }
    };
}

// single macros for every color
#[macro_export]
macro_rules! red {
    ($fmt:expr) => {
        $crate::color!(Color::Red, $fmt)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::color!(Color::Red, $fmt, $($arg)*)
    };
}
#[macro_export]
macro_rules! green {
    ($fmt:expr) => {
        $crate::color!(Color::Green, $fmt)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::color!(Color::Green, $fmt, $($arg)*)
    };
}
#[macro_export]
macro_rules! yellow {
    ($fmt:expr) => {
        $crate::color!(Color::Yellow, $fmt)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::color!(Color::Yellow, $fmt, $($arg)*)
    };
}
#[macro_export]
macro_rules! blue {
    ($fmt:expr) => {
        $crate::color!(Color::Blue, $fmt)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::color!(Color::Blue, $fmt, $($arg)*)
    };
}
#[macro_export]
macro_rules! magenta {
    ($fmt:expr) => {
        $crate::color!(Color::Magenta, $fmt)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::color!(Color::Magenta, $fmt, $($arg)*)
    };
}
#[macro_export]
macro_rules! cyan {
    ($fmt:expr) => {
        $crate::color!(Color::Cyan, $fmt)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::color!(Color::Cyan, $fmt, $($arg)*)
    };
}
#[macro_export]
macro_rules! white {
    ($fmt:expr) => {
        $crate::color!(Color::White, $fmt)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::color!(Color::White, $fmt, $($arg)*)
    };
}
