# frozen_string_literal: true

# A small inventory domain: modules for namespacing and mixins, classes
# with attr_ accessors, custom errors, a Comparable value object, an
# Enumerable collection, and a Struct - the shapes a Ruby preview should
# show beyond one loose method.

require "forwardable"
require "json"

module Warehouse
  VERSION = "2.1.0"

  Error = Class.new(StandardError)

  class OutOfStock < Error
    attr_reader :sku, :wanted, :available

    def initialize(sku, wanted, available)
      @sku = sku
      @wanted = wanted
      @available = available
      super("#{sku}: wanted #{wanted}, only #{available} left")
    end
  end

  class UnknownSku < Error
    def initialize(sku)
      super("no such sku: #{sku}")
    end
  end

  module Auditable
    def audit_log
      @audit_log ||= []
    end

    def record(event, **details)
      audit_log << { at: Time.now.utc.iso8601, event: event, **details }
      self
    end

    def last_event
      audit_log.last
    end
  end

  Money = Struct.new(:pennies) do
    include Comparable

    def self.from_pounds(pounds)
      new((pounds * 100).round)
    end

    def +(other)
      Money.new(pennies + other.pennies)
    end

    def *(count)
      Money.new(pennies * count)
    end

    def <=>(other)
      pennies <=> other.pennies
    end

    def to_s
      format("£%.2f", pennies / 100.0)
    end
  end

  class Item
    include Comparable

    attr_reader :sku, :name, :price
    attr_accessor :quantity

    def initialize(sku:, name:, price:, quantity: 0)
      @sku = sku
      @name = name
      @price = price
      @quantity = quantity
    end

    def value
      price * quantity
    end

    def in_stock?
      quantity.positive?
    end

    def <=>(other)
      sku <=> other.sku
    end

    def to_h
      { sku: sku, name: name, pennies: price.pennies, quantity: quantity }
    end

    def to_json(*args)
      to_h.to_json(*args)
    end
  end

  class Inventory
    include Enumerable
    include Auditable
    extend Forwardable

    def_delegators :@items, :size, :empty?, :key?

    def initialize(items = [])
      @items = items.to_h { |item| [item.sku, item] }
    end

    def each(&block)
      @items.each_value(&block)
    end

    def [](sku)
      @items.fetch(sku) { raise UnknownSku, sku }
    end

    def add(item)
      existing = @items[item.sku]
      if existing
        existing.quantity += item.quantity
      else
        @items[item.sku] = item
      end
      record(:added, sku: item.sku, quantity: item.quantity)
    end

    def reserve(sku, count)
      item = self[sku]
      raise OutOfStock.new(sku, count, item.quantity) if item.quantity < count

      item.quantity -= count
      record(:reserved, sku: sku, quantity: count)
      item
    end

    def total_value
      reduce(Money.new(0)) { |sum, item| sum + item.value }
    end

    def low_stock(threshold: 5)
      select { |item| item.quantity <= threshold }.sort
    end

    def to_json(*args)
      map(&:to_h).to_json(*args)
    end
  end
end

if __FILE__ == $PROGRAM_NAME
  inventory = Warehouse::Inventory.new(
    [
      Warehouse::Item.new(sku: "WID-1", name: "Widget", price: Warehouse::Money.from_pounds(4.50), quantity: 12),
      Warehouse::Item.new(sku: "GAD-7", name: "Gadget", price: Warehouse::Money.from_pounds(19.99), quantity: 2)
    ]
  )

  inventory.reserve("WID-1", 3)

  puts "total #{inventory.total_value}"
  puts "low: #{inventory.low_stock.map(&:sku).join(', ')}"
  puts inventory.last_event.inspect

  begin
    inventory.reserve("GAD-7", 5)
  rescue Warehouse::OutOfStock => e
    puts "refused: #{e.message}"
  end
end
