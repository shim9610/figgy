# Browser integration example

This crate is an example application adapter for the Rust chart renderer. It
demonstrates wasm-bindgen exports and a `<figgy-chart>` custom element that owns
a canvas, forwards pointer events, schedules frames and adapts async calls.

The library packages are `figgy-renderer` and `figgy-model`. This example keeps
the Cargo name `figgy` for its generated JS/WASM filenames and has `publish = false`.
It is not the JavaScript counterpart of every Rust API: the wrapper does **not**
expose the bounded radial, categorical or boxplot renderers. Those APIs are
demonstrated in the [native examples](../renderer/examples/README.md).

## Build and run

From the repository root, with Rust 1.99.0 and Node installed:

```bash
rustup target add wasm32-unknown-unknown
npx --yes wasm-pack@0.15.0 build crates/web --release --target web --locked
npx --yes serve crates/web -l 8142
```

Open <http://localhost:8142/> for the interactive histogram, or
<http://localhost:8142/timestamp-demo.html> for timestamp axes. Use a browser
with WebGPU enabled. Localhost is a secure context; remote hosting needs HTTPS.
Opening the HTML directly as a `file:` URL is not the supported flow.

The pages and [figgy-chart.js](figgy-chart.js) load the generated `pkg/` files.
Keep the JavaScript and WASM from the same build together. `pkg/` is generated,
not checked into the source snapshot.

| Source / guide | Purpose |
|---|---|
| [index.html](index.html) | Interactive histogram, bin/stroke controls and PNG output |
| [timestamp-demo.html](timestamp-demo.html) | Timestamp data and axis formatting |
| [figgy-chart.js](figgy-chart.js) | Example host lifecycle, events and Promise adaptation |
| [src/lib.rs](src/lib.rs) | wasm-bindgen bridge to the renderer |
| [WASM guide](../renderer/WASM.md) | Build, lifecycle, streaming, async export and wrapper methods |
| [JSON reference](SCHEMA.md) | The example wrapper's representation of the shared model |

## Linking a live demo from GitHub

GitHub repository READMEs display images and GIFs but do not execute JavaScript,
WASM or embedded iframes. An interactive chart needs its own page. The files
above can be hosted on an HTTPS static site such as GitHub Pages after building
the matching `pkg/`. Do not publish an empty `pkg/` or link to an unbuilt page.

Keep the gallery image linked to its source example; add a separate **Live demo**
link once a hosted URL has been deployed and checked. A repository HTML file link
opens source on GitHub, not a running chart. This guide does not claim that a
GitHub Pages deployment already exists.

[figgyplot.com Studio](https://figgyplot.com/) is an independently developed
application using the renderer; its deployment and UI are separate from this example.

## 한국어 안내

이 디렉터리는 Rust 렌더러를 브라우저에 연결하는 예제다. WASM으로 컴파일되는 렌더러와,
캔버스·이벤트·비동기 호출을 연결하는 이 래퍼, 별도 애플리케이션인 Studio를 구분한다.
`FiggyChart`나 `<figgy-chart>`가 모든 Rust 차트 API를 제공하는 것은 아니다.

위 명령으로 빌드하고 로컬 서버를 열면 히스토그램과 시간축 예제를 조작할 수 있다.
GitHub README 안에서는 JavaScript와 WASM을 실행할 수 없으므로, 직접 조작하는 데모는
별도 HTTPS 페이지에 올리고 링크로 연결한다. 이미지 클릭은 예제 코드로 연결하고
실행 페이지 링크는 따로 둔다.
