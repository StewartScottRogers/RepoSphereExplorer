package com.example.trading;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.math.BigDecimal;
import java.util.ArrayList;
import java.util.List;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

class OrderBookTest {

    private OrderBook book;

    @BeforeEach
    void setUp() {
        book = new OrderBook();
    }

    private static OrderBook.Order buy(String id, String price, int quantity) {
        return new OrderBook.Order(id, OrderBook.Side.BUY, new BigDecimal(price), quantity);
    }

    private static OrderBook.Order sell(String id, String price, int quantity) {
        return new OrderBook.Order(id, OrderBook.Side.SELL, new BigDecimal(price), quantity);
    }

    @Test
    @DisplayName("a resting order with nothing to match becomes the best price")
    void restingOrderSetsTheBest() {
        book.submit(buy("b1", "10.00", 5));

        assertEquals(new BigDecimal("10.00"), book.bestBid().orElseThrow());
        assertTrue(book.bestAsk().isEmpty());
    }

    @Test
    @DisplayName("a crossing order fills against what is resting")
    void crossingOrderFills() {
        book.submit(sell("s1", "10.00", 5));

        List<OrderBook.Fill> fills = book.submit(buy("b1", "10.00", 5));

        assertEquals(1, fills.size());
        assertEquals(5, fills.get(0).quantity());
        assertEquals("b1", fills.get(0).takerId());
        assertEquals("s1", fills.get(0).makerId());
    }

    @Test
    @DisplayName("listeners are told about every fill")
    void listenersHearFills() {
        List<OrderBook.Fill> heard = new ArrayList<>();
        book.addListener(heard::add);

        book.submit(sell("s1", "9.50", 3));
        book.submit(buy("b1", "9.50", 3));

        assertEquals(1, heard.size());
    }

    @Test
    @DisplayName("an order of no quantity is refused, not silently dropped")
    void zeroQuantityIsRefused() {
        assertThrows(IllegalArgumentException.class, () -> buy("b1", "10.00", 0));
    }
}
