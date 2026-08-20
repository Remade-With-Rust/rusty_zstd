fn main(){
  let mut h=0u64;
  for id in ["mr","dickens","samba","mozilla","xml","nci","sao","webster","osdb","ooffice"]{
    let Ok(f)=std::fs::read(format!("corpora/data/silesia/{id}")) else{continue};
    let s=&f[..f.len().min(8<<20)];
    for lv in [1,3,19]{
      let z=rusty_zstd::compress(s,lv).unwrap();
      for b in &z{ h=h.wrapping_mul(1099511628211).wrapping_add(*b as u64); }
      assert_eq!(rusty_zstd::decompress(&z).unwrap(), s, "{id} L{lv} roundtrip");
    }
  }
  println!("fnv {h:016x}");
}
