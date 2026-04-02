# Functions in Asatsuyu

Functions are declared with `fn`. Use `pub fn` to export.

The last expression in the body is the return value — no `return` keyword needed.

```asatsuyu
fn greet(name: String) -> String {
  "Hello, " <> name <> "!"
}

pub fn main() {
  println(greet("world"))
}
```

Use `async fn` for functions that call async Python APIs:

```asatsuyu
async fn fetch(url: String) -> String {
  let response = await get(url)
  response.text
}
```
