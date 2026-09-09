# order-book

A price-time priority limit order book: submit an order, get back the
fills it caused, and hear about every one of them as a listener.

## Using it

```java
OrderBook book = new OrderBook();
book.addListener(fill -> log.info("filled {}", fill));

book.submit(new OrderBook.Order("s1", OrderBook.Side.SELL, new BigDecimal("10.00"), 5));
List<OrderBook.Fill> fills =
    book.submit(new OrderBook.Order("b1", OrderBook.Side.BUY, new BigDecimal("10.00"), 5));
```

## Notes

- Prices are `BigDecimal`, never `double`. A book that loses a hundredth of
  a penny per fill is a book nobody can reconcile.
- An order of zero or negative quantity throws rather than resting. An
  order that can never fill is not an order.
- Matching is price first, then time: the oldest order at the best price
  fills first, which is what "price-time priority" means.

## Building

```bash
mvn test
mvn package
```

---

**This is a fixture.** It lives in `samples/java/` so the application has a
Java project to open, not just a Java file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
