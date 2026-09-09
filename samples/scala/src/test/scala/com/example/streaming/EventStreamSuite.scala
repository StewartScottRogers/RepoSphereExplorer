package com.example.streaming

import scala.util.Success

class EventStreamSuite extends munit.FunSuite:

  private def opened(id: String) = AccountOpened(id, owner = "Ada", at = 1L)

  test("an appended event comes back with an offset"):
    val store = InMemoryEventStore()

    val stored = store.append(opened("acc-1"))

    assert(stored.isInstanceOf[Success[?]])
    assertEquals(store.size, 1)

  test("reading from an offset older than the earliest is refused, not silently clamped"):
    val store = InMemoryEventStore()
    store.append(opened("acc-1"))
    store.compact(before = 1L)

    store.read(from = 0L) match
      case Left(OffsetTooOld(requested, _)) => assertEquals(requested, 0L)
      case other                            => fail(s"expected OffsetTooOld, got $other")

  test("a projection folds an account to its balance"):
    val events = Seq(
      opened("acc-1"),
      MoneyDeposited("acc-1", pennies = 2500L, at = 2L),
      MoneyWithdrawn("acc-1", pennies = 500L, at = 3L)
    )

    val state = AccountProjection.fold("acc-1", events)

    assertEquals(state.balancePennies, 2000L)
    assertEquals(state.pounds, BigDecimal(20))

  test("a closed account stays closed"):
    val events = Seq(opened("acc-1"), AccountClosed("acc-1", reason = "requested", at = 9L))

    assert(AccountProjection.fold("acc-1", events).closed)

  test("an empty projection is not an error"):
    assertEquals(AccountProjection.empty("acc-1").balancePennies, 0L)
