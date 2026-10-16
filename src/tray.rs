//! System tray icon.
//!
//! Linux uses GtkStatusIcon. `install` calls `gtk_init_check`, then
//! `gtk_status_icon_new`. Show and Quit are menu items. The window polls
//! `poll` on the UI thread, which runs `gtk_main_iteration`.
//!
//! Windows and macOS do not have a tray icon yet; `install` returns an
//! error there and the window works without one.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

static SHOW: AtomicBool = AtomicBool::new(false);
static QUIT: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Show,
    Quit,
}

pub struct Tray {
    #[cfg(target_os = "linux")]
    gtk: LinuxTray,
    #[cfg(windows)]
    _windows: WindowsTray,
}

pub fn install() -> io::Result<Tray> {
    #[cfg(target_os = "linux")]
    {
        Ok(Tray {
            gtk: LinuxTray::install()?,
        })
    }
    #[cfg(windows)]
    {
        Ok(Tray {
            _windows: WindowsTray::install()?,
        })
    }
    #[cfg(target_os = "macos")]
    {
        macos_install()?;
        Ok(Tray {})
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    Err(io::Error::other("no system tray on this platform"))
}

impl Tray {
    /// Show that something needs attention: on Linux the icon changes, and
    /// `blink` alternates it on every call. Off restores the normal icon.
    pub fn set_attention(&mut self, on: bool, blink: bool) {
        #[cfg(target_os = "linux")]
        self.gtk.set_attention(on, blink);
        #[cfg(not(target_os = "linux"))]
        let _ = (on, blink);
    }

    /// Whether the icon is on screen in a tray now, so hiding the window
    /// leaves a way to bring it back.
    pub fn shown(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.gtk
                .embedded
                // SAFETY: `status` is the live GtkStatusIcon made in `install`.
                .is_some_and(|embedded| unsafe { embedded(self.gtk.status) } != 0)
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    pub fn poll(&self) -> Option<Action> {
        #[cfg(target_os = "linux")]
        self.gtk.pump();
        #[cfg(windows)]
        windows_pump();
        #[cfg(target_os = "macos")]
        macos_pump();
        if QUIT.swap(false, Ordering::Relaxed) {
            Some(Action::Quit)
        } else if SHOW.swap(false, Ordering::Relaxed) {
            Some(Action::Show)
        } else {
            None
        }
    }
}

#[cfg(target_os = "linux")]
struct LinuxTray {
    events_pending: unsafe extern "C" fn() -> i32,
    iteration: unsafe extern "C" fn() -> i32,
    set_name: unsafe extern "C" fn(*mut libc::c_void, *const libc::c_char),
    set_tip: unsafe extern "C" fn(*mut libc::c_void, *const libc::c_char),
    /// Whether a tray took the icon; missing in very old GTK.
    embedded: Option<unsafe extern "C" fn(*mut libc::c_void) -> i32>,
    status: *mut libc::c_void,
    /// The icon now shown: false for the normal one.
    alert: bool,
    /// The tooltip while something waits, in the user's language.
    activity_tip: std::ffi::CString,
    _gtk: *mut libc::c_void,
    _object: *mut libc::c_void,
}

#[cfg(target_os = "linux")]
impl LinuxTray {
    fn install() -> io::Result<Self> {
        if !x11_tray_running() {
            return Err(io::Error::other(
                "no system tray is running on this desktop, so OpenIRC runs without a tray icon",
            ));
        }
        unsafe {
            let gtk = libc::dlopen(
                c"libgtk-3.so.0".as_ptr(),
                libc::RTLD_LAZY | libc::RTLD_GLOBAL,
            );
            if gtk.is_null() {
                return Err(io::Error::other("libgtk-3.so.0 is not available"));
            }
            let object = libc::dlopen(
                c"libgobject-2.0.so.0".as_ptr(),
                libc::RTLD_LAZY | libc::RTLD_GLOBAL,
            );
            if object.is_null() {
                return Err(io::Error::other("libgobject-2.0.so.0 is not available"));
            }
            let init: unsafe extern "C" fn(*mut i32, *mut *mut *mut libc::c_char) -> i32 =
                load(gtk, c"gtk_init_check")?;
            let new_icon: unsafe extern "C" fn() -> *mut libc::c_void =
                load(gtk, c"gtk_status_icon_new")?;
            let set_name: unsafe extern "C" fn(*mut libc::c_void, *const libc::c_char) =
                load(gtk, c"gtk_status_icon_set_from_icon_name")?;
            let set_tip: unsafe extern "C" fn(*mut libc::c_void, *const libc::c_char) =
                load(gtk, c"gtk_status_icon_set_tooltip_text")?;
            let set_visible: unsafe extern "C" fn(*mut libc::c_void, i32) =
                load(gtk, c"gtk_status_icon_set_visible")?;
            let menu_new: unsafe extern "C" fn() -> *mut libc::c_void = load(gtk, c"gtk_menu_new")?;
            let item_new: unsafe extern "C" fn(*const libc::c_char) -> *mut libc::c_void =
                load(gtk, c"gtk_menu_item_new_with_label")?;
            let append: unsafe extern "C" fn(*mut libc::c_void, *mut libc::c_void) =
                load(gtk, c"gtk_menu_shell_append")?;
            let show: unsafe extern "C" fn(*mut libc::c_void) = load(gtk, c"gtk_widget_show")?;
            let connect: unsafe extern "C" fn(
                *mut libc::c_void,
                *const libc::c_char,
                *const libc::c_void,
                *mut libc::c_void,
                *const libc::c_void,
                i32,
            ) -> u64 = load(object, c"g_signal_connect_data")?;
            let embedded: Option<unsafe extern "C" fn(*mut libc::c_void) -> i32> =
                load(gtk, c"gtk_status_icon_is_embedded").ok();
            let events_pending: unsafe extern "C" fn() -> i32 = load(gtk, c"gtk_events_pending")?;
            let iteration: unsafe extern "C" fn() -> i32 = load(gtk, c"gtk_main_iteration")?;
            if init(std::ptr::null_mut(), std::ptr::null_mut()) == 0 {
                return Err(io::Error::other("gtk_init_check failed"));
            }
            let status = new_icon();
            if status.is_null() {
                return Err(io::Error::other("gtk_status_icon_new failed"));
            }
            set_name(status, c"network-idle".as_ptr());
            set_tip(status, c"OpenIRC".as_ptr());
            set_visible(status, 1);
            let menu = menu_new();
            // Menu text in the user's language; a text with a NUL byte
            // (never in the catalogs) falls back to an empty label.
            let label = |text: &str| std::ffi::CString::new(text).unwrap_or_default();
            let show_label = label(crate::i18n::tr("Show OpenIRC"));
            let quit_label = label(crate::i18n::tr("Exit"));
            let show_item = item_new(show_label.as_ptr());
            let quit_item = item_new(quit_label.as_ptr());
            append(menu, show_item);
            append(menu, quit_item);
            show(show_item);
            show(quit_item);
            connect(
                show_item,
                c"activate".as_ptr(),
                show_clicked as *const _,
                std::ptr::null_mut(),
                std::ptr::null(),
                0,
            );
            connect(
                quit_item,
                c"activate".as_ptr(),
                quit_clicked as *const _,
                std::ptr::null_mut(),
                std::ptr::null(),
                0,
            );
            connect(
                status,
                c"activate".as_ptr(),
                show_clicked as *const _,
                std::ptr::null_mut(),
                std::ptr::null(),
                0,
            );
            connect(
                status,
                c"popup-menu".as_ptr(),
                popup_menu as *const _,
                menu,
                std::ptr::null(),
                0,
            );
            Ok(Self {
                events_pending,
                iteration,
                set_name,
                set_tip,
                embedded,
                status,
                alert: false,
                activity_tip: label(crate::i18n::tr("OpenIRC: new activity")),
                _gtk: gtk,
                _object: object,
            })
        }
    }

    fn set_attention(&mut self, on: bool, blink: bool) {
        let alert = on && (!blink || !self.alert);
        if alert == self.alert {
            return;
        }
        self.alert = alert;
        let (name, tip) = if alert {
            (c"mail-unread", self.activity_tip.as_c_str())
        } else if on {
            (c"network-idle", self.activity_tip.as_c_str())
        } else {
            (c"network-idle", c"OpenIRC")
        };
        unsafe {
            (self.set_name)(self.status, name.as_ptr());
            (self.set_tip)(self.status, tip.as_ptr());
        }
    }

    fn pump(&self) {
        unsafe {
            while (self.events_pending)() != 0 {
                (self.iteration)();
            }
        }
    }
}

#[cfg(target_os = "linux")]
unsafe fn load<T>(lib: *mut libc::c_void, name: &std::ffi::CStr) -> io::Result<T> {
    let symbol = unsafe { libc::dlsym(lib, name.as_ptr()) };
    if symbol.is_null() {
        Err(io::Error::other(format!(
            "missing {}",
            name.to_string_lossy()
        )))
    } else {
        Ok(unsafe { std::mem::transmute_copy(&symbol) })
    }
}

/// True when an X11 system tray (an owner of `_NET_SYSTEM_TRAY_S0`) is
/// running. GtkStatusIcon only works with one; without it GTK logs critical
/// warnings and the icon never appears.
#[cfg(target_os = "linux")]
fn x11_tray_running() -> bool {
    if std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    type OpenDisplay = unsafe extern "C" fn(*const libc::c_char) -> *mut libc::c_void;
    type InternAtom =
        unsafe extern "C" fn(*mut libc::c_void, *const libc::c_char, i32) -> libc::c_ulong;
    type SelectionOwner = unsafe extern "C" fn(*mut libc::c_void, libc::c_ulong) -> libc::c_ulong;
    type CloseDisplay = unsafe extern "C" fn(*mut libc::c_void) -> i32;
    unsafe {
        let x11 = libc::dlopen(c"libX11.so.6".as_ptr(), libc::RTLD_LAZY);
        if x11.is_null() {
            return false;
        }
        let found = (|| -> io::Result<bool> {
            let open: OpenDisplay = load(x11, c"XOpenDisplay")?;
            let intern: InternAtom = load(x11, c"XInternAtom")?;
            let owner: SelectionOwner = load(x11, c"XGetSelectionOwner")?;
            let close: CloseDisplay = load(x11, c"XCloseDisplay")?;
            let display = open(std::ptr::null());
            if display.is_null() {
                return Ok(false);
            }
            let atom = intern(display, c"_NET_SYSTEM_TRAY_S0".as_ptr(), 0);
            let running = atom != 0 && owner(display, atom) != 0;
            close(display);
            Ok(running)
        })()
        .unwrap_or(false);
        libc::dlclose(x11);
        found
    }
}

#[cfg(target_os = "linux")]
extern "C" fn show_clicked(_widget: *mut libc::c_void, _data: *mut libc::c_void) {
    SHOW.store(true, Ordering::Relaxed);
}

#[cfg(target_os = "linux")]
extern "C" fn quit_clicked(_widget: *mut libc::c_void, _data: *mut libc::c_void) {
    QUIT.store(true, Ordering::Relaxed);
}

#[cfg(target_os = "linux")]
extern "C" fn popup_menu(icon: *mut libc::c_void, button: u32, when: u32, menu: *mut libc::c_void) {
    unsafe {
        let gtk = libc::dlopen(
            c"libgtk-3.so.0".as_ptr(),
            libc::RTLD_LAZY | libc::RTLD_NOLOAD,
        );
        if gtk.is_null() {
            return;
        }
        type Popup = unsafe extern "C" fn(
            *mut libc::c_void,
            *mut libc::c_void,
            *mut libc::c_void,
            *const libc::c_void,
            *mut libc::c_void,
            u32,
            u32,
        );
        type Position = unsafe extern "C" fn(
            *mut libc::c_void,
            *mut i32,
            *mut i32,
            *mut i32,
            *mut libc::c_void,
        );
        let popup: Popup = match load(gtk, c"gtk_menu_popup") {
            Ok(popup) => popup,
            Err(_) => return,
        };
        let position: Position = match load(gtk, c"gtk_status_icon_position_menu") {
            Ok(position) => position,
            Err(_) => return,
        };
        popup(
            menu,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            position as *const _,
            icon,
            button,
            when,
        );
    }
}

#[cfg(windows)]
struct WindowsTray;

#[cfg(windows)]
impl WindowsTray {
    fn install() -> io::Result<Self> {
        Err(io::Error::other(
            "the Windows tray icon is not implemented yet",
        ))
    }
}

#[cfg(windows)]
fn windows_pump() {}

#[cfg(target_os = "macos")]
fn macos_install() -> io::Result<()> {
    Err(io::Error::other(
        "the macOS menu bar icon is not implemented yet",
    ))
}

#[cfg(target_os = "macos")]
fn macos_pump() {}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(target_os = "linux")]
    fn gtk_status_icon_symbols_exist() {
        unsafe {
            let gtk = libc::dlopen(c"libgtk-3.so.0".as_ptr(), libc::RTLD_LAZY);
            if gtk.is_null() {
                // No GTK 3 on this machine: the tray reports an error at run
                // time instead, which `install_returns_instead_of_panicking`
                // covers.
                eprintln!("skipped: libgtk-3.so.0 is not installed");
                return;
            }
            for name in [
                c"gtk_init_check".as_ptr(),
                c"gtk_status_icon_new".as_ptr(),
                c"gtk_status_icon_set_from_icon_name".as_ptr(),
                c"gtk_status_icon_set_tooltip_text".as_ptr(),
            ] {
                assert!(!libc::dlsym(gtk, name).is_null(), "missing gtk symbol");
            }
            libc::dlclose(gtk);
        }
    }

    #[test]
    fn install_returns_instead_of_panicking() {
        let _ = super::install();
    }
}
