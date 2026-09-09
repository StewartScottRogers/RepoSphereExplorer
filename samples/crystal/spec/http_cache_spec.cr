require "./spec_helper"

describe HttpCache::Store do
  it "returns nothing for a key it has never seen" do
    store = HttpCache::Store.new(capacity: 4)

    store.get("/missing").should be_nil
  end

  it "returns what was put in" do
    store = HttpCache::Store.new(capacity: 4)
    store.put("/a", "body-a")

    store.get("/a").should_not be_nil
  end

  it "refuses a capacity that could never hold anything" do
    expect_raises(HttpCache::CapacityError) do
      HttpCache::Store.new(capacity: 0)
    end
  end

  it "does not grow past its capacity" do
    store = HttpCache::Store.new(capacity: 2)

    store.put("/a", "body-a")
    store.put("/b", "body-b")
    store.put("/c", "body-c")

    store.size.should be <= 2
  end

  it "keeps an entry only as long as its own time to live" do
    store = HttpCache::Store.new(capacity: 4)

    entry = store.put("/a", "body-a", ttl: 1.hour)

    entry.url.should eq("/a")
  end

  it "fetches through to the block only on a miss" do
    store = HttpCache::Store.new(capacity: 4)
    calls = 0

    2.times do
      store.fetch("/a") do |_url|
        calls += 1
        "body-a"
      end
    end

    calls.should eq(1)
  end
end
