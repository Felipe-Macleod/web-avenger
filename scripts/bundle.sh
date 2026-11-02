#!/usr/bin/env bash
# Monta uma pasta autocontida (dist/web-avenger) com o executável + runtime do CEF.
set -euo pipefail

cd "$(dirname "$0")/.."
PROFILE="${PROFILE:-release}"
# O `cargo install` coloca o cargo-about aqui, que nem sempre está no PATH.
export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo-about >/dev/null; then
  echo "Falta o cargo-about (licenças de terceiros): cargo install --locked cargo-about --features cli" >&2
  exit 1
fi
cargo build --profile "$PROFILE"

TARGET_DIR="target/$PROFILE"
DIST="dist/web-avenger"

# Diretório do CEF: $CEF_PATH (exportado) ou o baixado pelo build do cef-dll-sys.
if [[ -n "${CEF_PATH:-}" && -f "$CEF_PATH/libcef.so" ]]; then
  CEF_DIR="$CEF_PATH"
else
  CEF_DIR="$(dirname "$(ls -t "$TARGET_DIR"/build/cef-dll-sys-*/out/*/libcef.so | head -n1)")"
fi
echo "Usando CEF de: $CEF_DIR"

rm -rf "$DIST"
mkdir -p "$DIST"
# Copia libs, .pak, icudtl.dat, snapshots do V8, locales/ etc. (ignora headers e artefatos de build).
for entry in "$CEF_DIR"/*; do
  case "$(basename "$entry")" in
    include|cmake|build|libcef_dll|*.txt|*.cmake|CMakeLists*|archive.json) continue ;;
  esac
  cp -r "$entry" "$DIST/"
done
cp "$TARGET_DIR/web-avenger" "$DIST/"
# O libcef.so vem com símbolos de debug (~1.4 GB); sem eles fica ~265 MB.
strip --strip-unneeded "$DIST/libcef.so"
# O sandbox SUID não é usado (rodamos com no_sandbox).
rm -f "$DIST/chrome-sandbox"

# Licenças: a do app, a do CEF (o CREDITS.html do CEF só cobre o Chromium) e a dos crates.
cp LICENSE "$DIST/LICENSE.txt"
cp licenses/CEF.txt "$DIST/CEF-LICENSE.txt"
cargo about generate --fail -c licenses/about.toml licenses/about.hbs -o "$DIST/THIRD_PARTY_LICENSES.html"

echo "Pronto: $DIST/web-avenger ($(du -sh "$DIST" | cut -f1))"
