//! Read-only native acceptance probe. Never sends input, shows/focuses a window,
//! registers a hook, changes a setting, or captures the screen.
use std::ffi::c_void;
type Hwnd = *mut c_void;
#[repr(C)]
#[derive(Default)]
struct Rect { left:i32, top:i32, right:i32, bottom:i32 }
#[link(name="user32")]
extern "system" {
    fn EnumWindows(callback:unsafe extern "system" fn(Hwnd,isize)->i32,data:isize)->i32;
    fn GetWindowThreadProcessId(hwnd:Hwnd,pid:*mut u32)->u32;
    fn IsWindowVisible(hwnd:Hwnd)->i32;
    fn IsZoomed(hwnd:Hwnd)->i32;
    fn GetWindowRect(hwnd:Hwnd,rect:*mut Rect)->i32;
    fn GetForegroundWindow()->Hwnd;
    fn GetWindowTextW(hwnd:Hwnd,text:*mut u16,max:i32)->i32;
    fn GetClassNameW(hwnd:Hwnd,text:*mut u16,max:i32)->i32;
}
struct Probe { pid:u32, found:Vec<String> }
unsafe extern "system" fn visit(hwnd:Hwnd,data:isize)->i32 {
    let probe=&mut *(data as *mut Probe);
    let mut pid=0;
    GetWindowThreadProcessId(hwnd,&mut pid);
    if pid != probe.pid { return 1; }
    let mut rect=Rect::default(); GetWindowRect(hwnd,&mut rect);
    let mut title=[0u16;256]; let n=GetWindowTextW(hwnd,title.as_mut_ptr(),256).max(0) as usize;
    let title=String::from_utf16_lossy(&title[..n]);
    let mut class=[0u16;256]; let n=GetClassNameW(hwnd,class.as_mut_ptr(),256).max(0) as usize;
    let class=String::from_utf16_lossy(&class[..n]);
    probe.found.push(format!("{{\"hwnd\":{},\"title\":{:?},\"class\":{:?},\"visible\":{},\"maximized\":{},\"foreground\":{},\"rect\":[{},{},{},{}]}}",hwnd as usize,title,class,IsWindowVisible(hwnd)!=0,IsZoomed(hwnd)!=0,GetForegroundWindow()==hwnd,rect.left,rect.top,rect.right,rect.bottom));
    1
}
fn main() {
    let mut probe=Probe {pid:std::env::args().nth(1).expect("PID").parse().expect("numeric PID"),found:Vec::new()};
    unsafe { EnumWindows(visit,(&mut probe as *mut Probe) as isize); }
    println!("{{\"pid\":{},\"windows\":[{}]}}",probe.pid,probe.found.join(","));
}
