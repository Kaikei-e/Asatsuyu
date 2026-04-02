# Pipelines

The `|>` operator passes the left-hand value as the first argument to the right-hand function:

```asatsuyu
[1, 2, 3, 4, 5]
|> list.filter(fn(n) { n > 2 })
|> list.map(fn(n) { n * 10 })
|> println
```

This reads top-to-bottom instead of inside-out. Combine with `list.map`, `list.filter`, and `list.length` for expressive data transformations.

String concatenation uses `<>`:

```asatsuyu
"Hello, " <> name <> "!"
```
