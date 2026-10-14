use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, ItemKey};
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let mut tf = lofty::read_from_path(&path).unwrap();
    println!("tag type {:?}, tags {}", tf.primary_tag_type(), tf.tags().len());
    let tag = tf.primary_tag_mut().expect("primary tag");
    println!("artist {:?}", tag.artist());
    let ok = tag.insert_text(ItemKey::Lyrics, "line one\nline two".to_string());
    println!("insert_text -> {ok}; has now {:?}", tag.get_string(ItemKey::Lyrics));
}
