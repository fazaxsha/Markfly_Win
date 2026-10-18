use std::io::Read;

fn main() {
    let mut md = String::new();
    std::io::stdin().read_to_string(&mut md).unwrap();
    print!("{}", markfly_core::render_preview(&md));
}
