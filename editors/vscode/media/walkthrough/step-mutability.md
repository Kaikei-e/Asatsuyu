# Mutability and Assignment

Bindings are **immutable by default**. Use `let mut` to opt in:

```asatsuyu
let x = 42          // immutable — cannot reassign
let mut counter = 0  // mutable — reassignment allowed
counter = counter + 1
```

The editor highlights reassignment sites with a distinct color, making mutation visible at a glance.

Only local variables can be mutable. Parameters and top-level bindings are always immutable.
