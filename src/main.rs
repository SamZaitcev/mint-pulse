use std::ffi::CString;
use std::fs::{self, File};
use std::io::Read;
use std::os::raw::{c_char, c_double, c_int, c_uint, c_void};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::Instant;

mod appearance;
use appearance::{Palette, Theme};

type Ptr = *mut c_void;
const N: usize = 40;
const CELL: i32 = 64;
const GAP: i32 = 4;
const WIDTH: i32 = CELL * 4 + GAP * 3;
const HEIGHT: i32 = 24;
const LABELS: [&str; 4] = ["CPU", "RAM", "SWAP", "SSD"];
const DEFAULT_THRESHOLDS: [f64; 4] = [90.0, 75.0, 50.0, 85.0];

#[repr(C)]
#[derive(Default, Debug)]
struct TextExtents {
    x_bearing: f64,
    y_bearing: f64,
    width: f64,
    height: f64,
    x_advance: f64,
    y_advance: f64,
}

#[link(name = "libcairo.so.2", kind = "dylib", modifiers = "+verbatim")]
extern "C" {
    fn cairo_image_surface_create(format: c_int, w: c_int, h: c_int) -> Ptr;
    fn cairo_surface_status(surface: Ptr) -> c_int;
    fn cairo_surface_write_to_png(surface: Ptr, path: *const c_char) -> c_int;
    fn cairo_surface_destroy(surface: Ptr);
    fn cairo_create(surface: Ptr) -> Ptr;
    fn cairo_destroy(cr: Ptr);
    fn cairo_status(cr: Ptr) -> c_int;
    fn cairo_scale(cr: Ptr, sx: c_double, sy: c_double);
    fn cairo_font_options_create() -> Ptr;
    fn cairo_font_options_destroy(options: Ptr);
    fn cairo_font_options_set_antialias(options: Ptr, antialias: c_int);
    fn cairo_set_font_options(cr: Ptr, options: Ptr);
    fn cairo_save(cr: Ptr);
    fn cairo_restore(cr: Ptr);
    fn cairo_set_source_rgba(cr: Ptr, r: c_double, g: c_double, b: c_double, a: c_double);
    fn cairo_rectangle(cr: Ptr, x: c_double, y: c_double, w: c_double, h: c_double);
    fn cairo_fill(cr: Ptr);
    fn cairo_clip(cr: Ptr);
    fn cairo_new_path(cr: Ptr);
    fn cairo_move_to(cr: Ptr, x: c_double, y: c_double);
    fn cairo_line_to(cr: Ptr, x: c_double, y: c_double);
    fn cairo_close_path(cr: Ptr);
    fn cairo_stroke(cr: Ptr);
    fn cairo_set_line_width(cr: Ptr, width: c_double);
    fn cairo_select_font_face(cr: Ptr, family: *const c_char, slant: c_int, weight: c_int);
    fn cairo_set_font_size(cr: Ptr, size: c_double);
    fn cairo_show_text(cr: Ptr, text: *const c_char);
    fn cairo_text_extents(cr: Ptr, text: *const c_char, extents: *mut TextExtents);
}

#[link(name = "libgio-2.0.so.0", kind = "dylib", modifiers = "+verbatim")]
extern "C" {
    fn g_file_new_for_path(path: *const c_char) -> Ptr;
    fn g_file_query_filesystem_info(file: Ptr, attrs: *const c_char, cancel: Ptr, error: *mut Ptr) -> Ptr;
    fn g_file_info_get_attribute_uint64(info: Ptr, attr: *const c_char) -> u64;
    fn g_file_info_has_attribute(info: Ptr, attr: *const c_char) -> c_int;
    fn g_application_new(id: *const c_char, flags: c_int) -> Ptr;
    fn g_application_register(app: Ptr, cancellable: Ptr, error: *mut Ptr) -> c_int;
    fn g_application_get_is_remote(app: Ptr) -> c_int;
    fn g_settings_schema_source_get_default() -> Ptr;
    fn g_settings_schema_source_lookup(source: Ptr, schema: *const c_char, recursive: c_int) -> Ptr;
    fn g_settings_schema_has_key(schema: Ptr, key: *const c_char) -> c_int;
    fn g_settings_schema_unref(schema: Ptr);
    fn g_settings_new_full(schema: Ptr, backend: Ptr, path: *const c_char) -> Ptr;
    fn g_settings_get_string(settings: Ptr, key: *const c_char) -> *mut c_char;
}

#[link(name = "libgobject-2.0.so.0", kind = "dylib", modifiers = "+verbatim")]
extern "C" {
    fn g_object_unref(object: Ptr);
}

#[link(name = "libglib-2.0.so.0", kind = "dylib", modifiers = "+verbatim")]
extern "C" {
    fn g_error_free(error: Ptr);
    fn g_free(memory: Ptr);
    fn g_main_loop_new(context: Ptr, running: c_int) -> Ptr;
    fn g_main_loop_run(main_loop: Ptr);
    fn g_main_loop_quit(main_loop: Ptr);
    fn g_main_loop_unref(main_loop: Ptr);
    fn g_timeout_add(ms: c_uint, callback: unsafe extern "C" fn(Ptr) -> c_int, data: Ptr) -> c_uint;
    fn g_source_remove(id: c_uint) -> c_int;
    fn g_unix_signal_add(signal: c_int, callback: unsafe extern "C" fn(Ptr) -> c_int, data: Ptr) -> c_uint;
}

#[link(name = "libgtk-3.so.0", kind = "dylib", modifiers = "+verbatim")]
extern "C" {
    fn gtk_init_check(argc: *mut c_int, argv: *mut *mut *mut c_char) -> c_int;
}

#[link(name = "libxapp.so.1", kind = "dylib", modifiers = "+verbatim")]
extern "C" {
    fn xapp_status_icon_new_with_name(name: *const c_char) -> Ptr;
    fn xapp_status_icon_set_icon_name(icon: Ptr, name: *const c_char);
    fn xapp_status_icon_set_tooltip_text(icon: Ptr, text: *const c_char);
    fn xapp_status_icon_set_visible(icon: Ptr, visible: c_int);
    fn xapp_status_icon_get_state(icon: Ptr) -> c_int;
    fn xapp_status_icon_get_icon_size(icon: Ptr) -> c_int;
}

fn cs(value: &str) -> CString {
    CString::new(value).expect("internal string contains NUL")
}

struct Object(Ptr);
impl Drop for Object {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                g_object_unref(self.0);
            }
        }
    }
}

struct Surface(Ptr);
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            cairo_surface_destroy(self.0);
        }
    }
}

struct Context(Ptr);
impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            cairo_destroy(self.0);
        }
    }
}

struct MainLoop(Ptr);
impl Drop for MainLoop {
    fn drop(&mut self) {
        unsafe {
            g_main_loop_unref(self.0);
        }
    }
}

struct ThemeDetector {
    settings: Vec<(Object, &'static str)>,
}

impl ThemeDetector {
    fn new() -> Self {
        let mut settings = Vec::new();
        unsafe {
            let source = g_settings_schema_source_get_default();

            if !source.is_null() {
                for (name, key) in [
                    ("org.cinnamon.theme", "name"),
                    ("org.cinnamon.desktop.interface", "gtk-theme"),
                    ("org.gnome.desktop.interface", "gtk-theme"),
                ] {
                    let schema = g_settings_schema_source_lookup(source, cs(name).as_ptr(), 1);

                    if schema.is_null() {
                        continue;
                    }

                    if g_settings_schema_has_key(schema, cs(key).as_ptr()) != 0 {
                        let object = g_settings_new_full(schema, ptr::null_mut(), ptr::null());

                        if !object.is_null() {
                            settings.push((Object(object), key));
                        }
                    }

                    g_settings_schema_unref(schema);
                }
            }
        }
        Self { settings }
    }

    fn resolve(&self, requested: Theme) -> Theme {
        if requested != Theme::Auto {
            return requested;
        }

        for (settings, key) in &self.settings {
            unsafe {
                let value = g_settings_get_string(settings.0, cs(key).as_ptr());

                if value.is_null() {
                    continue;
                }

                let name = std::ffi::CStr::from_ptr(value).to_string_lossy().into_owned();
                g_free(value.cast());

                if !name.is_empty() {
                    return Theme::from_name(&name);
                }
            }
        }
        Theme::Dark
    }
}

fn claim_instance() -> Result<Option<Object>, String> {
    unsafe {
        let app = g_application_new(cs("local.mintpulse.Indicator").as_ptr(), 0);

        if app.is_null() {
            return Err("Cannot create application identity".into());
        }

        let app = Object(app);
        let mut error = ptr::null_mut();
        let ok = g_application_register(app.0, ptr::null_mut(), &mut error);

        if !error.is_null() {
            g_error_free(error);
        }

        if ok == 0 {
            return Err("Cannot register application on session bus".into());
        }

        if g_application_get_is_remote(app.0) != 0 {
            Ok(None)
        } else {
            Ok(Some(app))
        }
    }
}

fn parse_thresholds(value: &str) -> Result<[f64; 4], String> {
    let values: Vec<f64> = value
        .split(',')
        .map(|v| v.parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| "Invalid thresholds")?;
    let thresholds: [f64; 4] = values.try_into().map_err(|_| "Expected CPU,RAM,SWAP,SSD thresholds")?;

    if thresholds.iter().any(|v| !v.is_finite() || !(1.0..100.0).contains(v)) {
        return Err("Thresholds must be in [1,100)".into());
    }

    Ok(thresholds)
}

fn pct(used: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| (100.0 * used as f64 / total as f64).clamp(0.0, 100.0))
}

#[derive(Clone, Copy, Debug)]
struct Cpu {
    total: u64,
    idle: u64,
}

fn parse_cpu(text: &str) -> Option<Cpu> {
    let mut words = text.lines().next()?.split_whitespace();

    if words.next()? != "cpu" {
        return None;
    }

    let counts: Vec<u64> = words.take(8).map(str::parse).collect::<Result<_, _>>().ok()?;

    if counts.len() < 4 {
        return None;
    }

    Some(Cpu {
        total: counts.iter().try_fold(0u64, |a, b| a.checked_add(*b))?,
        idle: counts[3].checked_add(*counts.get(4).unwrap_or(&0))?,
    })
}

fn cpu_pct(old: Option<Cpu>, now: Cpu) -> Option<f64> {
    let old = old?;
    let total = now.total.checked_sub(old.total)?;
    let idle = now.idle.checked_sub(old.idle)?;
    pct(total.checked_sub(idle)?, total)
}

#[derive(Debug)]
struct Memory {
    ram: Option<f64>,
    swap: Option<f64>,
    total: u64,
    used: u64,
    swap_total: u64,
    swap_used: u64,
}

fn parse_memory(text: &str) -> Option<Memory> {
    let mut values = [None; 4];
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let index = match words.next() {
            Some("MemTotal:") => 0,
            Some("MemAvailable:") => 1,
            Some("SwapTotal:") => 2,
            Some("SwapFree:") => 3,
            _ => continue,
        };
        let value = words.next()?.parse::<u64>().ok()?.checked_mul(1024)?;

        if words.next()? != "kB" {
            return None;
        }

        values[index] = Some(value);
    }
    let total = values[0]?;

    if total == 0 {
        return None;
    }

    let used = total.checked_sub(values[1]?)?;
    let swap_total = values[2]?;
    let swap_used = swap_total.checked_sub(values[3]?)?;
    Some(Memory {
        ram: pct(used, total),
        swap: pct(swap_used, swap_total),
        total,
        used,
        swap_total,
        swap_used,
    })
}

#[derive(Debug)]
struct Disk {
    used: u64,
    free: u64,
    size: u64,
}

impl Disk {
    fn percent(&self) -> Option<f64> {
        let available_total = self.used.checked_add(self.free)?;

        if self.size == 0 || available_total > self.size {
            return None;
        }

        pct(self.used, available_total)
    }
}

fn disk(path: &Path) -> Option<Disk> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    unsafe {
        let file = g_file_new_for_path(path.as_ptr());

        if file.is_null() {
            return None;
        }

        let mut error = ptr::null_mut();
        let info = g_file_query_filesystem_info(
            file,
            cs("filesystem::used,filesystem::free,filesystem::size").as_ptr(),
            ptr::null_mut(),
            &mut error,
        );
        g_object_unref(file);

        if !error.is_null() {
            g_error_free(error);
        }

        if info.is_null() {
            return None;
        }

        let read = |name: &str| {
            let attr = cs(name);

            if g_file_info_has_attribute(info, attr.as_ptr()) == 0 {
                None
            } else {
                Some(g_file_info_get_attribute_uint64(info, attr.as_ptr()))
            }
        };
        let result = (|| {
            Some(Disk {
                used: read("filesystem::used")?,
                free: read("filesystem::free")?,
                size: read("filesystem::size")?,
            })
        })();
        g_object_unref(info);
        result
    }
}

#[derive(Clone)]
struct History {
    values: [Option<f64>; N],
    next: usize,
    len: usize,
}

impl History {
    fn new() -> Self {
        Self {
            values: [None; N],
            next: 0,
            len: 0,
        }
    }

    fn push(&mut self, value: Option<f64>) {
        self.values[self.next] = value.filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 100.0));
        self.next = (self.next + 1) % N;
        self.len = (self.len + 1).min(N);
    }

    fn at(&self, i: usize) -> Option<f64> {
        self.values[((if self.len == N { self.next } else { 0 }) + i) % N]
    }

    fn latest(&self) -> Option<f64> {
        if self.len == 0 {
            None
        } else {
            self.at(self.len - 1)
        }
    }
}

unsafe fn color(cr: Ptr, rgb: u32, alpha: f64) {
    cairo_set_source_rgba(
        cr,
        ((rgb >> 16) & 255) as f64 / 255.0,
        ((rgb >> 8) & 255) as f64 / 255.0,
        (rgb & 255) as f64 / 255.0,
        alpha,
    );
}

unsafe fn trace(cr: Ptr, history: &History, x: f64, filled: bool) {
    cairo_new_path(cr);

    let step = CELL as f64 / (N - 1) as f64;
    let first = x + (N - history.len) as f64 * step;
    let mut open = false;
    let mut start = 0.0;
    let mut last = 0.0;
    for i in 0..=history.len {
        if let Some(value) = if i < history.len { history.at(i) } else { None } {
            let px = first + i as f64 * step;
            let py = (HEIGHT - 1) as f64 * (1.0 - value / 100.0);

            if open {
                cairo_line_to(cr, px, py);
            } else {
                cairo_move_to(cr, px, py);
                start = px;
                open = true;
            }

            last = px;
        } else {
            if open && filled {
                cairo_line_to(cr, last, HEIGHT as f64);
                cairo_line_to(cr, start, HEIGHT as f64);
                cairo_close_path(cr);
            }

            open = false;
        }
    }
}

unsafe fn text_extents(cr: Ptr, value: &str, size: f64) -> TextExtents {
    cairo_select_font_face(cr, cs("DejaVu Sans").as_ptr(), 0, 1);
    cairo_set_font_size(cr, size);
    let mut extents = TextExtents::default();
    cairo_text_extents(cr, cs(value).as_ptr(), &mut extents);
    extents
}

unsafe fn label(cr: Ptr, value: &str, edge: f64, right: bool, size: f64) -> (f64, f64) {
    let extents = text_extents(cr, value, size);
    let left = if right { edge - extents.width } else { edge };
    let x = left - extents.x_bearing;
    let baseline = HEIGHT as f64 - 2.0 - (extents.y_bearing + extents.height);
    cairo_move_to(cr, x, baseline);
    cairo_show_text(cr, cs(value).as_ptr());
    (left, left + extents.width)
}

fn render(
    path: &Path,
    histories: &[History; 4],
    thresholds: &[f64; 4],
    palette: Palette,
    height: i32,
) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    unsafe {
        let width = (WIDTH as f64 * height as f64 / HEIGHT as f64).round() as i32;
        let surface = Surface(cairo_image_surface_create(0, width, height));

        if cairo_surface_status(surface.0) != 0 {
            return Err("Cannot allocate chart surface".into());
        }

        let context = Context(cairo_create(surface.0));
        let cr = context.0;
        cairo_scale(cr, width as f64 / WIDTH as f64, height as f64 / HEIGHT as f64);

        let font_options = cairo_font_options_create();
        cairo_font_options_set_antialias(font_options, 2);
        cairo_set_font_options(cr, font_options);
        cairo_font_options_destroy(font_options);
        for (i, history) in histories.iter().enumerate() {
            let x = (i as i32 * (CELL + GAP)) as f64;

            cairo_save(cr);
            cairo_rectangle(cr, x, 0.0, CELL as f64, HEIGHT as f64);
            cairo_clip(cr);
            color(cr, palette.background, 1.0);
            cairo_rectangle(cr, x, 0.0, CELL as f64, HEIGHT as f64);
            cairo_fill(cr);
            cairo_save(cr);
            cairo_rectangle(cr, x, 0.0, CELL as f64, HEIGHT as f64);
            cairo_clip(cr);
            trace(cr, history, x, true);
            color(cr, palette.graphs[i], palette.fill_alpha);
            cairo_fill(cr);
            trace(cr, history, x, false);
            color(cr, palette.graphs[i], 1.0);
            cairo_set_line_width(cr, 1.0);
            cairo_stroke(cr);

            cairo_rectangle(
                cr,
                x,
                0.0,
                CELL as f64,
                (HEIGHT - 1) as f64 * (1.0 - thresholds[i] / 100.0),
            );
            cairo_clip(cr);
            trace(cr, history, x, true);
            color(cr, palette.critical, 0.95);
            cairo_fill(cr);
            trace(cr, history, x, false);
            color(cr, palette.critical, 1.0);
            cairo_stroke(cr);
            cairo_restore(cr);
            let value = history.latest().map_or_else(|| "—".into(), |v| format!("{:.0}%", v));
            let mut value_size = 8.0;
            let title_width = text_extents(cr, LABELS[i], 7.0).width;
            while value_size > 6.0 && title_width + text_extents(cr, &value, value_size).width > CELL as f64 - 10.0 {
                value_size -= 0.5;
            }
            color(cr, palette.text, 1.0);
            label(cr, LABELS[i], x + 3.0, false, 7.0);
            label(cr, &value, x + CELL as f64 - 3.0, true, value_size);
            cairo_restore(cr);
        }
        let drawing_status = cairo_status(cr);

        if drawing_status != 0 {
            return Err(format!("Cannot draw chart: Cairo {}", drawing_status));
        }

        let status = cairo_surface_write_to_png(surface.0, path.as_ptr());

        if status != 0 {
            Err(format!("Cannot write chart image: Cairo {}", status))
        } else {
            Ok(())
        }
    }
}

struct Frames {
    dir: PathBuf,
    paths: [PathBuf; 2],
    staging: PathBuf,
}

impl Frames {
    fn new() -> Result<Self, String> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let dir = PathBuf::from(format!("/dev/shm/mint-pulse-{}-{}", std::process::id(), nonce));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            paths: [dir.join("a.png"), dir.join("b.png")],
            staging: dir.join("staging.png"),
            dir,
        })
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_file(&self.staging);
        let _ = fs::remove_dir(&self.dir);
    }
}

struct State {
    icon: Ptr,
    main_loop: Ptr,
    frames: Frames,
    history: [History; 4],
    previous: Option<Cpu>,
    disk_path: PathBuf,
    thresholds: [f64; 4],
    updates: usize,
    limit: usize,
    started: Instant,
    interval_min: f64,
    interval_max: f64,
    interval_sum: f64,
    last: Instant,
    errors: usize,
    theme: Theme,
    detector: ThemeDetector,
    height: Option<i32>,
    fatal_error: bool,
}

impl State {
    fn update(&mut self) -> Result<(), String> {
        let now = Instant::now();

        if self.updates > 0 {
            let elapsed = now.duration_since(self.last).as_secs_f64() * 1000.0;
            self.interval_min = self.interval_min.min(elapsed);
            self.interval_max = self.interval_max.max(elapsed);
            self.interval_sum += elapsed;
        }

        self.last = now;
        let mut buffer = [0u8; 512];
        let cpu = File::open("/proc/stat")
            .and_then(|mut file| file.read(&mut buffer))
            .ok()
            .and_then(|len| std::str::from_utf8(&buffer[..len]).ok())
            .and_then(parse_cpu);
        let cpu_value = cpu.and_then(|current| cpu_pct(self.previous, current));
        self.previous = cpu;
        let memory = fs::read_to_string("/proc/meminfo").ok().and_then(|s| parse_memory(&s));
        let storage = disk(&self.disk_path);
        self.errors += usize::from(cpu.is_none()) + usize::from(memory.is_none()) + usize::from(storage.is_none());
        let values = [
            cpu_value,
            memory.as_ref().and_then(|m| m.ram),
            memory.as_ref().and_then(|m| m.swap),
            storage.as_ref().and_then(Disk::percent),
        ];
        for (history, value) in self.history.iter_mut().zip(values) {
            history.push(value);
        }
        let path = &self.frames.paths[self.updates % 2];
        let height = self.height.unwrap_or_else(|| {
            if self.icon.is_null() {
                HEIGHT
            } else {
                unsafe { xapp_status_icon_get_icon_size(self.icon).clamp(16, 96) }
            }
        });
        render(
            &self.frames.staging,
            &self.history,
            &self.thresholds,
            self.detector.resolve(self.theme).palette(),
            height,
        )?;

        fs::rename(&self.frames.staging, path).map_err(|e| format!("Cannot publish chart: {}", e))?;

        if !self.icon.is_null() {
            let p = path.to_str().ok_or("Invalid runtime path")?;
            let gib = |v| v as f64 / 1073741824.0;
            let mut tooltip = format!(
                "CPU: {}\n",
                values[0].map_or_else(|| "—".into(), |v| format!("{:.1}%", v))
            );
            tooltip += &memory.map_or_else(
                || "RAM/SWAP: нет данных\n".into(),
                |m| {
                    format!(
                        "RAM: {:.1} / {:.1} GiB\nSWAP: {}\n",
                        gib(m.used),
                        gib(m.total),
                        if m.swap_total == 0 {
                            "отключён".into()
                        } else {
                            format!("{:.1} / {:.1} GiB", gib(m.swap_used), gib(m.swap_total))
                        }
                    )
                },
            );
            tooltip += &storage.map_or_else(
                || "SSD: нет данных".into(),
                |d| {
                    format!(
                        "SSD: занято {:.1} / {:.1} GiB; доступно {:.1} GiB",
                        gib(d.used),
                        gib(d.size),
                        gib(d.free)
                    )
                },
            );
            tooltip += &format!("\nОбновление 2 с · история около 80 с\nКрасные вершины: CPU >{:.0}%, RAM >{:.0}%, SWAP >{:.0}%, SSD >{:.0}%",
                self.thresholds[0], self.thresholds[1], self.thresholds[2], self.thresholds[3]);
            unsafe {
                xapp_status_icon_set_icon_name(self.icon, cs(p).as_ptr());
                xapp_status_icon_set_tooltip_text(self.icon, cs(&tooltip).as_ptr());
            }
        }

        self.updates += 1;
        Ok(())
    }
}

unsafe extern "C" fn tick(data: Ptr) -> c_int {
    let state = &mut *data.cast::<State>();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.update()));

    if !matches!(result, Ok(Ok(()))) {
        state.fatal_error = true;
        match result {
            Ok(Err(error)) => eprintln!("mint-pulse: {}", error),
            _ => eprintln!("mint-pulse: chart update panicked; stopping safely"),
        }
        g_main_loop_quit(state.main_loop);
        return 1;
    }

    if state.limit > 0 && state.updates >= state.limit {
        g_main_loop_quit(state.main_loop);
    }

    1
}

unsafe extern "C" fn stop(data: Ptr) -> c_int {
    g_main_loop_quit(data);
    1
}

fn demo(dir: &Path, thresholds: &[f64; 4], palette: Palette, height: i32) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for critical in [false, true] {
        let mut histories = std::array::from_fn(|_| History::new());
        for (metric, history) in histories.iter_mut().enumerate() {
            for i in 0..N {
                let t = i as f64;
                let value = if critical {
                    match metric {
                        0 => 65.0 + 34.0 * (t * 0.32).sin().abs(),
                        1 => 82.0 + 16.0 * (t * 0.16).sin().abs(),
                        2 => 73.0 + 25.0 * (t * 0.22).sin().abs(),
                        _ => 85.0 + t * 0.28,
                    }
                } else {
                    match metric {
                        0 => 26.0 + 21.0 * (t * 0.42).sin() + 8.0 * (t * 1.2).cos(),
                        1 => 54.0 + 5.0 * (t * 0.15).sin(),
                        2 => 8.0 + 3.0 * (t * 0.12).sin(),
                        _ => 76.0 + t * 0.02,
                    }
                };
                history.push(Some(value));
            }
        }
        render(
            &dir.join(if critical { "critical.png" } else { "normal.png" }),
            &histories,
            thresholds,
            palette,
            height,
        )?;
    }
    for (name, value) in [
        ("low", Some(10.0)),
        ("high", Some(97.0)),
        ("missing", None),
        ("0", Some(0.0)),
        ("9", Some(9.0)),
        ("10", Some(10.0)),
        ("75", Some(75.0)),
        ("99", Some(99.0)),
        ("100", Some(100.0)),
    ] {
        let mut histories = std::array::from_fn(|_| History::new());
        for history in &mut histories {
            for _ in 0..N {
                history.push(value);
            }
        }
        render(
            &dir.join(format!("test-{}.png", name)),
            &histories,
            thresholds,
            palette,
            height,
        )?;
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut thresholds = DEFAULT_THRESHOLDS;
    let mut limit = 0;
    let mut bench = false;
    let mut demo_dir = None;
    let mut disk_path = None;
    let mut theme = Theme::Auto;
    let mut height = None;
    let mut i = 0;
    while i < args.len() {
        let argument = &args[i];

        if argument == "--help" || argument == "-h" {
            println!("Mint Pulse {}\n\nCPU, RAM, SWAP and filesystem graphs for the Cinnamon XApp tray.\n\nOptions:\n  --theme auto|dark|light|high-contrast  Color scheme (default: auto)\n  --thresholds CPU,RAM,SWAP,SSD        Red peaks (default: 90,75,50,85)\n  --threshold N                       Override all four thresholds\n  --path PATH                         Filesystem to monitor (default: home)\n  --height N                          Render height, 16..96 px (default: tray size)\n  --demo DIRECTORY                    Write example PNGs without a display\n  --bench --ticks N                   Run N updates without a tray\n  --ticks N                           Stop after N updates (0: continuous)\n  --version                           Show version\n  --help                              Show this help\n\nUpdates every 2 seconds. Horizontal Cinnamon panel with XApp Status Applet required.", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }

        if argument == "--version" || argument == "-V" {
            println!("mint-pulse {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }

        if argument == "--bench" {
            bench = true;
            i += 1;
            continue;
        }

        i += 1;
        let value = args.get(i).ok_or("Missing option value")?;
        match argument.as_str() {
            "--threshold" => thresholds = parse_thresholds(&[value.as_str(); 4].join(","))?,
            "--thresholds" => thresholds = parse_thresholds(value)?,
            "--ticks" => limit = value.parse().map_err(|_| "Invalid tick count")?,
            "--demo" => demo_dir = Some(PathBuf::from(value)),
            "--path" => disk_path = Some(PathBuf::from(value)),
            "--theme" => theme = Theme::parse(value)?,
            "--height" => {
                let size: i32 = value.parse().map_err(|_| "Invalid icon height")?;

                if !(16..=96).contains(&size) {
                    return Err("Icon height must be 16..96".into());
                }

                height = Some(size);
            }
            _ => return Err(format!("Unknown option: {}", argument)),
        }
        i += 1;
    }

    if let Some(dir) = demo_dir {
        return demo(
            &dir,
            &thresholds,
            ThemeDetector::new().resolve(theme).palette(),
            height.unwrap_or(HEIGHT),
        );
    }

    let disk_path = disk_path
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .ok_or("HOME is unavailable; use --path")?;

    if bench && limit < 2 {
        return Err("Bench needs --ticks >=2".into());
    }

    let _instance = if bench {
        None
    } else {
        match claim_instance()? {
            Some(app) => Some(app),
            None => {
                println!("Mint Pulse is already running");
                return Ok(());
            }
        }
    };
    unsafe {
        if !bench && gtk_init_check(ptr::null_mut(), ptr::null_mut()) == 0 {
            return Err("Cannot connect to desktop display".into());
        }

        let loop_owner = MainLoop(g_main_loop_new(ptr::null_mut(), 0));
        let main_loop = loop_owner.0;

        if main_loop.is_null() {
            return Err("Cannot create event loop".into());
        }

        let icon_owner = Object(if bench {
            ptr::null_mut()
        } else {
            xapp_status_icon_new_with_name(cs("Mint Pulse").as_ptr())
        });
        let icon = icon_owner.0;

        if !bench && icon.is_null() {
            return Err("Cannot create tray indicator".into());
        }

        let mut state = Box::new(State {
            icon,
            main_loop,
            frames: Frames::new()?,
            history: std::array::from_fn(|_| History::new()),
            previous: None,
            disk_path,
            thresholds,
            updates: 0,
            limit,
            started: Instant::now(),
            interval_min: f64::INFINITY,
            interval_max: 0.0,
            interval_sum: 0.0,
            last: Instant::now(),
            errors: 0,
            theme,
            detector: ThemeDetector::new(),
            height,
            fatal_error: false,
        });
        state.update()?;

        if !icon.is_null() {
            xapp_status_icon_set_visible(icon, 1);
        }

        let data = (&mut *state as *mut State).cast::<c_void>();
        let timer = g_timeout_add(2000, tick, data);
        let signals = [
            g_unix_signal_add(2, stop, main_loop),
            g_unix_signal_add(15, stop, main_loop),
        ];

        if limit == 0 || state.updates < limit {
            g_main_loop_run(main_loop);
        }

        g_source_remove(timer);
        for id in signals {
            g_source_remove(id);
        }
        let native = if icon.is_null() {
            -1
        } else {
            xapp_status_icon_get_state(icon)
        };

        if !icon.is_null() {
            xapp_status_icon_set_visible(icon, 0);
        }

        let (min, max, mean) = if state.updates < 2 {
            (0.0, 0.0, 0.0)
        } else {
            (
                state.interval_min,
                state.interval_max,
                state.interval_sum / (state.updates - 1) as f64,
            )
        };
        println!("{{\"updates\":{},\"errors\":{},\"seconds\":{:.6},\"interval_ms\":{{\"min\":{:.3},\"max\":{:.3},\"mean\":{:.3}}},\"xapp_state\":{},\"frame_dir\":\"{}\"}}",
            state.updates,state.errors,state.started.elapsed().as_secs_f64(),min,max,mean,native,state.frames.dir.display());

        if state.fatal_error {
            return Err("Chart updates stopped".into());
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("mint-pulse: {}", error);
        std::process::exit(1);
    }
}
