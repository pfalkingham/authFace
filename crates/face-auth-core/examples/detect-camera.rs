//! Diagnostic: show which V4L2 nodes look like IR cameras and which one
//! authentication would actually use.
fn main() {
    println!("Scanning /sys/class/video4linux ...\n");
    for entry in std::fs::read_dir("/sys/class/video4linux").into_iter().flatten().flatten() {
        let node = entry.file_name().to_string_lossy().to_string();
        let name = std::fs::read_to_string(entry.path().join("name"))
            .unwrap_or_default().trim().to_string();
        let path = format!("/dev/{node}");
        let ir_name = face_auth_core::capture::name_suggests_ir(&name);
        let opens = face_auth_core::capture::Camera::open(&path);
        println!("{path:<14} name={name:?}");
        println!("  {:<22} {}", "name looks like IR:", ir_name);
        match opens {
            Ok(_) => println!("  {:<22} yes (GREY capture)", "opens as IR device:"),
            Err(e) => println!("  {:<22} no — {e}", "opens as IR device:"),
        }
        println!();
    }
    println!("enumerate_ir_cameras() -> {:?}", face_auth_core::capture::enumerate_ir_cameras());
    println!("detect_ir_camera()     -> {:?}", face_auth_core::capture::detect_ir_camera());
}
