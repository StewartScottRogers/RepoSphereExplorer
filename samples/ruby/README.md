# warehouse

Stock, money and audit trails for a small warehouse. Money is kept in
pennies, errors name the stock keeping unit that caused them, and anything
that includes `Auditable` records what happened to it.

## Using it

```ruby
inventory = Warehouse::Inventory.new
inventory.add(Warehouse::Item.new("SKU-1", "Widget", Warehouse::Money.new(250)), 40)

inventory.take("SKU-1", 2)
```

## Notes

- `Money` is a `Struct` of pennies. A warehouse that stores prices as
  floats will eventually fail to reconcile, and nobody will know when it
  started.
- `UnknownSku` and `OutOfStock` are separate errors: "we do not sell that"
  and "we sold out" need different answers from a caller.
- `Auditable` is a mixin rather than a base class, so an item can be
  audited without being anything in particular.

## Developing

```bash
bundle install
bundle exec rake        # rubocop, then rspec
```

---

**This is a fixture.** It lives in `samples/ruby/` so the application has a
Ruby project to open, not just a Ruby file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
