#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("FontLoader 只能在 Windows 上运行。");
}

#[cfg(windows)]
mod windows_app {
    use std::{
        ffi::{c_void, OsString},
        fs,
        mem::size_of,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::{Path, PathBuf},
        ptr::addr_of,
    };

    use windows::{
        core::{w, Error, Result, PCWSTR, PWSTR},
        Win32::{
            Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
            Graphics::Gdi::{AddFontResourceW, RemoveFontResourceW},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Controls::{
                    InitCommonControlsEx, CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDIS_SELECTED,
                    CDRF_NEWFONT, CDRF_NOTIFYITEMDRAW, ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX,
                    LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVITEMW, LVM_DELETEALLITEMS,
                    LVM_GETITEMCOUNT, LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_INSERTITEMW,
                    LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE, LVNI_SELECTED, LVN_KEYDOWN,
                    LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_REPORT, LVS_SHOWSELALWAYS,
                    NMHDR, NMLVCUSTOMDRAW, NMLVKEYDOWN, NM_CUSTOMDRAW,
                },
                Shell::{
                    DefSubclassProc, DragAcceptFiles, DragFinish, DragQueryFileW,
                    SetWindowSubclass, HDROP,
                },
                WindowsAndMessaging::{
                    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
                    GetWindowLongPtrW, LoadCursorW, MessageBoxW, MoveWindow, PostQuitMessage,
                    RegisterClassW, SendMessageTimeoutW, SendMessageW, SetWindowLongPtrW,
                    ShowWindow, TranslateMessage, CW_USEDEFAULT, GWLP_USERDATA, HWND_BROADCAST,
                    IDC_ARROW, MB_ICONERROR, MB_OK, MSG, SMTO_ABORTIFHUNG, SMTO_BLOCK, SW_SHOW,
                    WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WM_DROPFILES,
                    WM_FONTCHANGE, WM_NCDESTROY, WM_NOTIFY, WM_SIZE, WNDCLASSW, WS_CHILD,
                    WS_EX_CLIENTEDGE, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
                },
            },
        },
    };

    const APP_CLASS: PCWSTR = w!("TemporaryFontLoaderWindow");
    const APP_TITLE: PCWSTR = w!("Font Loader");
    const LIST_CLASS: PCWSTR = w!("SysListView32");
    const VK_BACKSPACE: u16 = 0x08;
    const VK_DELETE: u16 = 0x2E;
    const LIST_SUBCLASS_ID: usize = 1;

    /// Stores the original UTF-16 path so it can be passed to
    /// `RemoveFontResourceW` without a lossy Unicode conversion.
    struct LoadedFont {
        path: Vec<u16>,
    }

    #[derive(Default)]
    struct AppState {
        list: HWND,
        fonts: Vec<LoadedFont>,
    }

    impl AppState {
        fn release_selected(&mut self, owner: HWND) {
            let mut selected = selected_rows(self.list);
            if selected.is_empty() {
                return;
            }

            // Remove from the end so the row indexes remain valid.
            selected.sort_unstable_by(|left, right| right.cmp(left));
            let mut changed = false;
            let mut failed = false;

            for index in selected {
                let Some(font) = self.fonts.get(index) else {
                    continue;
                };

                let removed = unsafe { RemoveFontResourceW(PCWSTR(font.path.as_ptr())).as_bool() };
                if removed {
                    self.fonts.remove(index);
                    changed = true;
                } else {
                    failed = true;
                }
            }

            if changed {
                refresh_list(self);
                broadcast_font_change();
            }

            if failed {
                unsafe {
                    MessageBoxW(
                        Some(owner),
                        w!("有一个或多个字体无法卸载，已保留在列表中。"),
                        APP_TITLE,
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
        }

        fn release_all(&mut self) {
            if self.fonts.is_empty() {
                return;
            }

            for font in &self.fonts {
                unsafe {
                    let _ = RemoveFontResourceW(PCWSTR(font.path.as_ptr()));
                }
            }
            self.fonts.clear();
            broadcast_font_change();
        }
    }

    pub fn run() -> Result<()> {
        unsafe {
            let common_controls = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_LISTVIEW_CLASSES,
            };
            if !InitCommonControlsEx(&common_controls).as_bool() {
                return Err(Error::from_win32());
            }

            let module = GetModuleHandleW(None)?;
            let instance = HINSTANCE(module.0);
            let window_class = WNDCLASSW {
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: APP_CLASS,
                lpfnWndProc: Some(window_proc),
                ..Default::default()
            };

            if RegisterClassW(&window_class) == 0 {
                return Err(Error::from_win32());
            }

            let window = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                APP_CLASS,
                APP_TITLE,
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                360,
                520,
                None,
                None,
                Some(instance),
                None,
            )?;

            let state = Box::into_raw(Box::new(AppState::default()));
            SetWindowLongPtrW(window, GWLP_USERDATA, state as isize);
            if let Err(error) = create_list_view(window) {
                let _ = DestroyWindow(window);
                return Err(error);
            }

            let _ = ShowWindow(window, SW_SHOW);

            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        Ok(())
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_SIZE => {
                unsafe { resize_list_view(window, lparam) };
                LRESULT(0)
            }
            WM_DROPFILES => {
                unsafe { add_dropped_files(window, HDROP(wparam.0 as *mut c_void)) };
                LRESULT(0)
            }
            WM_NOTIFY => unsafe { handle_list_notification(window, lparam) },
            WM_CLOSE => {
                unsafe {
                    let _ = DestroyWindow(window);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                unsafe { free_app_state(window) };
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            // A failed WM_CREATE can skip WM_DESTROY on some paths. This is a
            // harmless second cleanup after a normal destroy because the user
            // data is cleared by free_app_state.
            WM_NCDESTROY => {
                unsafe { free_app_state(window) };
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            }
            _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
        }
    }

    unsafe fn create_list_view(window: HWND) -> Result<()> {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let list = unsafe {
            CreateWindowExW(
                WS_EX_CLIENTEDGE,
                LIST_CLASS,
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(LVS_REPORT) | WINDOW_STYLE(LVS_SHOWSELALWAYS),
                0,
                0,
                0,
                0,
                Some(window),
                None,
                Some(instance),
                None,
            )?
        };

        let Some(state) = (unsafe { state_from_window(window) }) else {
            return Err(Error::from_win32());
        };
        state.list = list;

        unsafe {
            SendMessageW(
                list,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                Some(WPARAM(0)),
                Some(LPARAM(
                    (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize,
                )),
            );
        }

        let mut heading: Vec<u16> = "已临时加载的字体文件"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut column = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH,
            cx: 320,
            pszText: PWSTR(heading.as_mut_ptr()),
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                Some(WPARAM(0)),
                Some(LPARAM((&mut column as *mut LVCOLUMNW) as isize)),
            );

            // DragAcceptFiles is applied to both windows because the list view
            // occupies the client area and is normally the drop target.
            DragAcceptFiles(window, true);
            DragAcceptFiles(list, true);
            let _ = SetWindowSubclass(
                list,
                Some(list_subclass_proc),
                LIST_SUBCLASS_ID,
                window.0 as usize,
            );
        }

        Ok(())
    }

    unsafe extern "system" fn list_subclass_proc(
        list: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _subclass_id: usize,
        parent: usize,
    ) -> LRESULT {
        if message == WM_DROPFILES {
            unsafe {
                add_dropped_files(HWND(parent as *mut c_void), HDROP(wparam.0 as *mut c_void));
            }
            return LRESULT(0);
        }

        unsafe { DefSubclassProc(list, message, wparam, lparam) }
    }

    unsafe fn resize_list_view(window: HWND, lparam: LPARAM) {
        let Some(state) = (unsafe { state_from_window(window) }) else {
            return;
        };
        if state.list.is_invalid() {
            return;
        }

        let width = (lparam.0 as u32 & 0xffff) as i32;
        let height = ((lparam.0 as u32 >> 16) & 0xffff) as i32;
        unsafe {
            let _ = MoveWindow(state.list, 0, 0, width, height, true);
            SendMessageW(
                state.list,
                LVM_SETCOLUMNWIDTH,
                Some(WPARAM(0)),
                Some(LPARAM(width as isize)),
            );
        }
    }

    unsafe fn handle_list_notification(window: HWND, lparam: LPARAM) -> LRESULT {
        if lparam.0 == 0 {
            return LRESULT(0);
        }

        let Some(state) = (unsafe { state_from_window(window) }) else {
            return LRESULT(0);
        };
        let header = unsafe { &*(lparam.0 as *const NMHDR) };
        if header.hwndFrom != state.list {
            return LRESULT(0);
        }

        if header.code == NM_CUSTOMDRAW {
            let custom_draw = unsafe { &mut *(lparam.0 as *mut NMLVCUSTOMDRAW) };
            if custom_draw.nmcd.dwDrawStage == CDDS_PREPAINT {
                return LRESULT(CDRF_NOTIFYITEMDRAW as isize);
            }

            if custom_draw.nmcd.dwDrawStage == CDDS_ITEMPREPAINT {
                // Keep the native highlight so Ctrl/Shift and marquee selection
                // stay clear; paint only non-selected rows in alternating colors.
                if !custom_draw.nmcd.uItemState.contains(CDIS_SELECTED) {
                    custom_draw.clrTextBk = if custom_draw.nmcd.dwItemSpec % 2 == 0 {
                        COLORREF(0x00FF_FFFF)
                    } else {
                        COLORREF(0x00F4_F4F4)
                    };
                }
                return LRESULT(CDRF_NEWFONT as isize);
            }
        }

        if header.code == LVN_KEYDOWN {
            let key_down = lparam.0 as *const NMLVKEYDOWN;
            let key = unsafe { addr_of!((*key_down).wVKey).read_unaligned() };
            if key == VK_DELETE || key == VK_BACKSPACE {
                state.release_selected(window);
                return LRESULT(0);
            }
        }

        LRESULT(0)
    }

    unsafe fn add_dropped_files(window: HWND, hdrop: HDROP) {
        let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
        let mut font_paths = Vec::new();
        let mut dropped_folder_count = 0usize;

        for index in 0..count {
            let length = unsafe { DragQueryFileW(hdrop, index, None) } as usize;
            let mut path = vec![0u16; length + 1];
            unsafe {
                DragQueryFileW(hdrop, index, Some(&mut path));
            }

            let dropped_path = path_from_wide(&path);
            if dropped_path.is_dir() {
                dropped_folder_count += 1;
                collect_fonts_in_folder(&dropped_path, &mut font_paths);
            } else {
                // Directly dropped files keep the original behavior: let GDI
                // decide whether the file is a supported font resource.
                font_paths.push(path);
            }
        }

        unsafe { DragFinish(hdrop) };

        if font_paths.is_empty() && dropped_folder_count != 0 {
            unsafe {
                MessageBoxW(
                    Some(window),
                    w!("文件夹及其子文件夹中没有找到可加载的字体文件。"),
                    APP_TITLE,
                    MB_OK | MB_ICONERROR,
                );
            }
            return;
        }

        let mut changed = false;
        for path in font_paths {
            changed |= unsafe { load_font_file(window, path) };
        }
        if changed {
            broadcast_font_change();
        }
    }

    /// Adds one resource through the same public GDI API used by the original
    /// FontLoader executable. A positive return value means GDI added one or
    /// more font faces to the current session's system font table.
    unsafe fn load_font_file(window: HWND, path: Vec<u16>) -> bool {
        if unsafe { AddFontResourceW(PCWSTR(path.as_ptr())) } > 0 {
            if let Some(state) = unsafe { state_from_window(window) } {
                state.fonts.push(LoadedFont { path });
                insert_last_row(state);
                return true;
            }

            // Do not leave an untracked system font behind if the main window
            // is already being torn down.
            unsafe {
                let _ = RemoveFontResourceW(PCWSTR(path.as_ptr()));
            }
            return false;
        }

        unsafe {
            MessageBoxW(
                Some(window),
                PCWSTR(path.as_ptr()),
                w!("无法加载此字体文件"),
                MB_OK | MB_ICONERROR,
            );
        }
        false
    }

    fn path_from_wide(path: &[u16]) -> PathBuf {
        let length = path
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(path.len());
        PathBuf::from(OsString::from_wide(&path[..length]))
    }

    fn path_to_wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn collect_fonts_in_folder(folder: &Path, font_paths: &mut Vec<Vec<u16>>) {
        let Ok(entries) = fs::read_dir(folder) else {
            return;
        };

        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            // Avoid following directory links, which could otherwise create a
            // recursive loop while walking a dropped folder.
            if file_type.is_symlink() {
                continue;
            }

            let path = entry.path();
            if file_type.is_dir() {
                collect_fonts_in_folder(&path, font_paths);
            } else if file_type.is_file() && is_supported_font_file(&path) {
                font_paths.push(path_to_wide(&path));
            }
        }
    }

    fn is_supported_font_file(path: &Path) -> bool {
        let Some(extension) = path.extension() else {
            return false;
        };
        let extension = extension.to_string_lossy();
        ["ttf", "otf", "ttc", "fon", "fnt", "fot"]
            .iter()
            .any(|supported| extension.eq_ignore_ascii_case(supported))
    }

    fn selected_rows(list: HWND) -> Vec<usize> {
        let mut result = Vec::new();
        let mut row = -1i32;

        loop {
            row = unsafe {
                SendMessageW(
                    list,
                    LVM_GETNEXTITEM,
                    Some(WPARAM(row as usize)),
                    Some(LPARAM(LVNI_SELECTED as isize)),
                )
                .0 as i32
            };
            if row == -1 {
                break;
            }
            result.push(row as usize);
        }

        result
    }

    fn refresh_list(state: &mut AppState) {
        unsafe {
            SendMessageW(
                state.list,
                LVM_DELETEALLITEMS,
                Some(WPARAM(0)),
                Some(LPARAM(0)),
            );
        }
        for index in 0..state.fonts.len() {
            insert_row(state, index);
        }
    }

    fn insert_last_row(state: &mut AppState) {
        let index = state.fonts.len() - 1;
        insert_row(state, index);
    }

    fn insert_row(state: &mut AppState, index: usize) {
        let font = &mut state.fonts[index];
        let current_count = unsafe {
            SendMessageW(
                state.list,
                LVM_GETITEMCOUNT,
                Some(WPARAM(0)),
                Some(LPARAM(0)),
            )
            .0 as i32
        };
        let mut item = LVITEMW {
            mask: LVIF_TEXT,
            iItem: current_count,
            iSubItem: 0,
            pszText: PWSTR(font.path.as_mut_ptr()),
            ..Default::default()
        };

        unsafe {
            SendMessageW(
                state.list,
                LVM_INSERTITEMW,
                Some(WPARAM(0)),
                Some(LPARAM((&mut item as *mut LVITEMW) as isize)),
            );
        }
    }

    fn broadcast_font_change() {
        unsafe {
            // Synchronously notify every responsive top-level window, while
            // allowing Font Loader to continue if another application is hung.
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_FONTCHANGE,
                WPARAM(0),
                LPARAM(0),
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                1_000,
                None,
            );
        }
    }

    unsafe fn state_from_window<'a>(window: HWND) -> Option<&'a mut AppState> {
        let raw = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut AppState;
        unsafe { raw.as_mut() }
    }

    unsafe fn free_app_state(window: HWND) {
        let raw = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut AppState;
        if raw.is_null() {
            return;
        }

        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            let mut state = Box::from_raw(raw);
            state.release_all();
        }
    }
}

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    windows_app::run()
}
