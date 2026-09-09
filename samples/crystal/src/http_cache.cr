# An in-memory HTTP response cache with expiry and a bounded size, in the
# shape Crystal encourages: modules for namespacing, structs for values,
# classes for the things that hold state, and everything type-annotated.

require "http/client"
require "json"

module HttpCache
  VERSION = "0.3.1"

  # How a stored response can stop being usable.
  enum Staleness
    Fresh
    Expired
    Evicted
  end

  class CacheError < Exception
  end

  class CapacityError < CacheError
    getter capacity : Int32

    def initialize(@capacity : Int32)
      super("cache capacity must be positive, got #{@capacity}")
    end
  end

  struct Entry
    include JSON::Serializable

    getter url : String
    getter body : String
    getter stored_at : Time
    getter ttl : Time::Span

    def initialize(@url : String, @body : String, @ttl : Time::Span, @stored_at : Time = Time.utc)
    end

    def expired?(now : Time = Time.utc) : Bool
      now - stored_at > ttl
    end

    def age : Time::Span
      Time.utc - stored_at
    end
  end

  # A least-recently-used cache of HTTP bodies.
  class Store
    getter capacity : Int32
    getter hits : Int32 = 0
    getter misses : Int32 = 0

    def initialize(@capacity : Int32 = 32)
      raise CapacityError.new(@capacity) unless @capacity > 0
      @entries = {} of String => Entry
      @order = [] of String
    end

    def size : Int32
      @entries.size
    end

    def put(url : String, body : String, ttl : Time::Span = 5.minutes) : Entry
      evict_oldest if @entries.size >= @capacity && !@entries.has_key?(url)
      entry = Entry.new(url, body, ttl)
      @entries[url] = entry
      touch(url)
      entry
    end

    def get(url : String) : Entry?
      entry = @entries[url]?
      if entry.nil?
        @misses += 1
        return nil
      end

      if entry.expired?
        @entries.delete(url)
        @order.delete(url)
        @misses += 1
        return nil
      end

      @hits += 1
      touch(url)
      entry
    end

    def fetch(url : String, & : String -> String) : String
      cached = get(url)
      return cached.body if cached

      body = yield url
      put(url, body)
      body
    end

    def state_of(url : String) : Staleness
      entry = @entries[url]?
      return Staleness::Evicted if entry.nil?
      entry.expired? ? Staleness::Expired : Staleness::Fresh
    end

    def hit_rate : Float64
      total = @hits + @misses
      total.zero? ? 0.0 : @hits.to_f / total
    end

    private def touch(url : String) : Nil
      @order.delete(url)
      @order << url
    end

    private def evict_oldest : Nil
      oldest = @order.shift?
      @entries.delete(oldest) if oldest
    end
  end
end

store = HttpCache::Store.new(capacity: 2)
store.put("https://example.com/a", "first body", 1.second)
store.put("https://example.com/b", "second body")
store.fetch("https://example.com/c") { |url| "fetched #{url}" }

puts "size: #{store.size}"
puts "a is #{store.state_of("https://example.com/a")}"
puts "hit rate: #{store.hit_rate.round(2)}"
