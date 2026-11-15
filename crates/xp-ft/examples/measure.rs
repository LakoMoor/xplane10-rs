//! Port glyph check: `measure <font> <px> <text>…` prints advances and the line's coverage.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let face = xp_ft::Face::open(std::path::Path::new(&a[1]), a[2].parse().unwrap()).unwrap();
    for t in &a[3..] {
        let adv: Vec<f64> = t.chars().map(|c| face.glyph(c, xp_ft::LOAD_TARGET_LIGHT).unwrap().advance).collect();
        println!("{t:?} sum {} {:?}", adv.iter().sum::<f64>(), adv);
        let b = xp_ft::render_line(&face, t, xp_ft::LOAD_TARGET_LIGHT).unwrap();
        for y in 0..b.height {
            let row: Vec<String> = (0..b.width).map(|x| format!("{:4}", 255 - b.coverage[y * b.width + x] as i32)).collect();
            println!("{}", row.join(""));
        }
    }
}
