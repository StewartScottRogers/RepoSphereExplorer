package com.example.trading;

import java.math.BigDecimal;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.Deque;
import java.util.List;
import java.util.Objects;
import java.util.Optional;
import java.util.TreeMap;

/**
 * A price-time priority order book: limit orders rest at a price level,
 * incoming orders match against the best opposing level first.
 */
public class OrderBook {

    public enum Side {
        BUY,
        SELL
    }

    public interface Listener {
        void onFill(Fill fill);
    }

    public static final class Order {
        private final String id;
        private final Side side;
        private final BigDecimal price;
        private int quantity;

        public Order(String id, Side side, BigDecimal price, int quantity) {
            this.id = Objects.requireNonNull(id, "id");
            this.side = Objects.requireNonNull(side, "side");
            this.price = Objects.requireNonNull(price, "price");
            if (quantity <= 0) {
                throw new IllegalArgumentException("quantity must be positive");
            }
            this.quantity = quantity;
        }

        public String id() {
            return id;
        }

        public int quantity() {
            return quantity;
        }

        @Override
        public String toString() {
            return side + " " + quantity + " @ " + price;
        }
    }

    public record Fill(String takerId, String makerId, BigDecimal price, int quantity) {
    }

    private final TreeMap<BigDecimal, Deque<Order>> bids =
            new TreeMap<>(Comparator.reverseOrder());
    private final TreeMap<BigDecimal, Deque<Order>> asks = new TreeMap<>();
    private final List<Listener> listeners = new ArrayList<>();

    public void addListener(Listener listener) {
        listeners.add(Objects.requireNonNull(listener));
    }

    public Optional<BigDecimal> bestBid() {
        return bids.isEmpty() ? Optional.empty() : Optional.of(bids.firstKey());
    }

    public Optional<BigDecimal> bestAsk() {
        return asks.isEmpty() ? Optional.empty() : Optional.of(asks.firstKey());
    }

    public List<Fill> submit(Order order) {
        List<Fill> fills = new ArrayList<>();
        TreeMap<BigDecimal, Deque<Order>> opposing = order.side == Side.BUY ? asks : bids;

        while (order.quantity > 0 && !opposing.isEmpty()) {
            BigDecimal bestPrice = opposing.firstKey();
            if (!crosses(order, bestPrice)) {
                break;
            }
            Deque<Order> level = opposing.get(bestPrice);
            Order maker = level.peekFirst();
            int traded = Math.min(order.quantity, maker.quantity);
            order.quantity -= traded;
            maker.quantity -= traded;
            Fill fill = new Fill(order.id, maker.id, bestPrice, traded);
            fills.add(fill);
            listeners.forEach(listener -> listener.onFill(fill));
            if (maker.quantity == 0) {
                level.pollFirst();
            }
            if (level.isEmpty()) {
                opposing.remove(bestPrice);
            }
        }

        if (order.quantity > 0) {
            rest(order);
        }
        return fills;
    }

    private boolean crosses(Order order, BigDecimal bestPrice) {
        return order.side == Side.BUY
                ? order.price.compareTo(bestPrice) >= 0
                : order.price.compareTo(bestPrice) <= 0;
    }

    private void rest(Order order) {
        TreeMap<BigDecimal, Deque<Order>> book = order.side == Side.BUY ? bids : asks;
        book.computeIfAbsent(order.price, price -> new ArrayDeque<>()).addLast(order);
    }

    public static void main(String[] args) {
        OrderBook book = new OrderBook();
        book.addListener(fill -> System.out.println("fill: " + fill));
        book.submit(new Order("m1", Side.SELL, new BigDecimal("10.25"), 100));
        book.submit(new Order("t1", Side.BUY, new BigDecimal("10.50"), 60));
        System.out.println("best ask now " + book.bestAsk().orElse(null));
    }
}
