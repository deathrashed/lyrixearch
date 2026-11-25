pkgname=lyrixearch
pkgver=0.1.0
pkgrel=1
pkgdesc='Find lyrics for your music and embed them in the files'
arch=('x86_64')
url='https://github.com/deathrashed/lyrixearch'
license=('MIT')
depends=('alsa-lib' 'gtk3' 'libayatana-appindicator' 'librsvg' 'openssl' 'webkit2gtk-4.1' 'xdotool')
makedepends=('cargo')
options=('!lto')

build() {
  cd "$startdir"
  CARGO_TARGET_DIR="$srcdir/target" cargo build --locked --release -p lyrixearch -p lyrixearch-gui
}

package() {
  cd "$startdir"
  install -Dm755 "$srcdir/target/release/lyrixearch" "$pkgdir/usr/bin/lyrixearch"
  install -Dm755 "$srcdir/target/release/lyrixearch-gui" "$pkgdir/usr/bin/lyrixearch-gui"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  XDG_DATA_HOME="$pkgdir/usr/share" CARGO_TARGET_DIR="$srcdir/target" bash scripts/linux-desktop-entry.sh release --package
}
