//! Print the port's per-glyph advances and line measure for strings: `measure <font> <px> <text>…`.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let face = xp_ft::Face::open(std::path::Path::new(&a[1]), a[2].parse().unwrap()).unwrap();
    for t in &a[3..] {
        let adv: Vec<f64> = t.chars().map(|c| face.glyph(c, xp_ft::LOAD_TARGET_LIGHT).unwrap().advance).collect();
        println!("{t:?} sum {} {:?}", adv.iter().sum::<f64>(), adv);
    }
}
