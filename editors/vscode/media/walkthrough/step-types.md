# Types and Pattern Matching

Define algebraic data types with named constructors:

```asatsuyu
type Shape {
  Circle(radius: Float)
  Rectangle(width: Float, height: Float)
}
```

Use `match` to decompose values. The compiler ensures all variants are handled:

```asatsuyu
fn area(s: Shape) -> Float {
  match s {
    Circle(r) -> 3.14159 * r * r
    Rectangle(w, h) -> w * h
  }
}
```

Failures use `Result`, not exceptions. Null is replaced by `Option`.
