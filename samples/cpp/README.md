# linalg

A small dense matrix template whose dimensions are checked before they are
used.

## Using it

```cpp
linalg::Matrix<double> a(2, 3);
linalg::Matrix<double> b(3, 2);

auto product = a * b;                    // 2x2
auto identity = linalg::Identity::of(3);

try {
    auto bad = a * a;                    // 2x3 by 2x3
} catch (const linalg::DimensionMismatch& err) {
    std::cerr << err.what() << '\n';     // names both shapes
}
```

## Notes

- Dimensions are values, not template parameters. A matrix whose shape is
  known only after a file has been read is the ordinary case, and encoding
  it in the type would make that case impossible.
- A mismatch throws `DimensionMismatch` with **both** shapes in the
  message. "Bad dimensions" is a message nobody can debug.
- Storage is one `std::vector`, row-major, so a row is contiguous and the
  common traversal is the cache-friendly one.

## Building

```bash
cmake -B build && cmake --build build && ctest --test-dir build
```

---

**This is a fixture.** It lives in `samples/cpp/` so the application has a
C++ project to open, not just a C++ file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
