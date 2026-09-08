//! Exercise the actual HWND editor through the VST3 host boundary.
use super::*;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::UpdateWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

struct AcceptEdits;
impl Class for AcceptEdits {
    type Interfaces = (IComponentHandler,);
}
impl IComponentHandlerTrait for AcceptEdits {
    unsafe fn beginEdit(&self, _: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn performEdit(&self, _: ParamID, _: ParamValue) -> tresult {
        kResultOk
    }
    unsafe fn endEdit(&self, _: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn restartComponent(&self, _: i32) -> tresult {
        kResultOk
    }
}

struct Parent(HWND);
impl Drop for Parent {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

fn frame(child: HWND) {
    unsafe {
        SendMessageW(child, WM_TIMER, Some(WPARAM(1)), Some(LPARAM(0)));
        let _ = UpdateWindow(child);
        let mut message = MSG::default();
        for _ in 0..256 {
            if !PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[test]
fn native_editor_types_delay_toggles_bypass_and_preserves_audio_on_hide() {
    let parent = Parent(
        unsafe {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("Pump host test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                1000,
                700,
                None,
                None,
                None,
                None,
            )
        }
        .expect("real host parent"),
    );
    let _ = unsafe { ShowWindow(parent.0, SW_SHOW) };
    let shared = Arc::new(PumpVst3Shared::new());
    let controller = PumpVst3Controller::new(Arc::clone(&shared));
    let handler = ComWrapper::new(AcceptEdits)
        .to_com_ptr::<IComponentHandler>()
        .unwrap();
    assert_eq!(
        unsafe { controller.setComponentHandler(handler.as_ptr()) },
        kResultOk
    );
    let view = unsafe { ComPtr::from_raw(controller.createView(ViewType::kEditor)) }
        .expect("Pump custom editor");
    assert_eq!(
        unsafe { view.attached(parent.0 .0, kPlatformTypeHWND) },
        kResultOk
    );
    let child = unsafe { GetWindow(parent.0, GW_CHILD) }.expect("embedded child HWND");
    frame(child);
    let mut rect = ViewRect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    assert_eq!(unsafe { view.getSize(&mut rect) }, kResultOk);
    let scale = rect.right as f32 / crate::gui::WINDOW_WIDTH as f32;
    let click = |x: f32, y: f32| {
        let point = LPARAM(
            ((x * scale).round() as isize & 0xffff)
                | (((y * scale).round() as isize & 0xffff) << 16),
        );
        unsafe {
            SendMessageW(child, WM_LBUTTONDOWN, Some(WPARAM(1)), Some(point));
            SendMessageW(child, WM_LBUTTONUP, Some(WPARAM(0)), Some(point));
        }
        frame(child);
    };
    let key = |character: u16, code: i16, modifiers: i16| {
        let result = unsafe { view.onKeyDown(character, code, modifiers) };
        let _ = unsafe { view.onKeyUp(character, code, modifiers) };
        frame(child);
        result
    };

    // Plain-click Delay supports native text commits and host editing keys.
    let division = shared.params.sync_division();
    click(193.0, 39.0);
    assert_eq!(key('a' as u16, 0, 4), kResultOk);
    unsafe {
        SendMessageW(child, WM_CHAR, Some(WPARAM('7' as usize)), Some(LPARAM(0)));
    }
    assert_eq!(key(0, 4, 0), kResultOk);
    assert_eq!(shared.params.delay_beats(), 7);
    assert_eq!(key(0, 12, 0), kResultOk);
    assert_eq!(key(0, 14, 1), kResultOk);
    assert_eq!(
        shared.params.delay_beats(),
        4,
        "up adds one, Shift-down subtracts four"
    );
    assert_eq!(
        shared.params.sync_division(),
        division,
        "Delay arrows must not change division"
    );

    click(570.0, 383.0);
    assert!(shared.params.bypassed());
    assert_eq!(key(' ' as u16, 7, 0), kResultOk);
    assert!(!shared.params.bypassed(), "focused bypass supports Space");
    unsafe {
        SendMessageW(child, WM_CHAR, Some(WPARAM(32)), Some(LPARAM(0)));
    }
    assert!(
        !shared.params.bypassed(),
        "text commits must not double-activate buttons"
    );

    shared.params.set_mix(0.37);
    for hidden in [SW_HIDE, SW_MINIMIZE] {
        let _ = unsafe { view.onFocus(0) };
        let _ = unsafe { ShowWindow(parent.0, hidden) };
        frame(child);
        assert_eq!(shared.params.delay_beats(), 4);
        assert!((shared.params.mix() - 0.37).abs() < 1e-6);
        assert!(!shared.params.bypassed());
        let _ = unsafe { ShowWindow(parent.0, SW_RESTORE) };
        frame(child);
    }
    rect.right = (960.0 * scale).round() as i32;
    rect.bottom = (600.0 * scale).round() as i32;
    assert_eq!(unsafe { view.checkSizeConstraint(&mut rect) }, kResultOk);
    assert_eq!(unsafe { view.onSize(&mut rect) }, kResultOk);
    assert_eq!(unsafe { view.removed() }, kResultOk);
    assert!(!unsafe { IsWindow(Some(child)) }.as_bool());
    assert_eq!(
        unsafe { view.attached(parent.0 .0, kPlatformTypeHWND) },
        kResultOk
    );
    let reopened = unsafe { GetWindow(parent.0, GW_CHILD) }.expect("reopened child HWND");
    frame(reopened);
    assert!(unsafe { IsWindow(Some(reopened)) }.as_bool());
    assert_eq!(shared.params.delay_beats(), 4);
    assert!((shared.params.mix() - 0.37).abs() < 1e-6);
    assert_eq!(unsafe { view.removed() }, kResultOk);
}
