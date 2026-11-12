fn main() {
    let acf = xp_core::acf::Acf::load(std::env::args().nth(1).unwrap()).unwrap();
    let l = xp_core::weights::Loading::from_acf(&acf);
    println!("{:?}", l);
    let mut w = xp_core::btnwin::load(6, "W", (1024.0, 768.0)).unwrap().unwrap().widgets;
    l.apply(&mut w);
    for x in &w { println!("{:?} {} {} {:?} {:?} {}", x.kind, x.x, x.y, x.key, x.items, x.cond); }
}
