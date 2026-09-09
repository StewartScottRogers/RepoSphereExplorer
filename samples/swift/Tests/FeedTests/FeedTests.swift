import Foundation
import Testing

@testable import Feed

@Suite("Feed items")
struct FeedItemTests {

    private func item(_ id: String) -> FeedItem {
        FeedItem(id: id, title: "Item \(id)", url: URL(string: "https://example.com/\(id)")!)
    }

    @Test("an item round-trips through JSON")
    func roundTrip() throws {
        let original = item("a")

        let data = try JSONEncoder().encode(original)
        let restored = try JSONDecoder().decode(FeedItem.self, from: data)

        #expect(restored == original)
    }

    @Test("items are hashable, so a set removes duplicates")
    func hashable() {
        #expect(Set([item("a"), item("a"), item("b")]).count == 2)
    }
}

@Suite("The cache")
struct InMemoryFeedCacheTests {

    @Test("what goes in comes back out")
    func storeAndLoad() async throws {
        let cache = InMemoryFeedCache()
        let items = [FeedItem(id: "a", title: "A", url: URL(string: "https://example.com/a")!)]

        try await cache.save(items)
        let loaded = try await cache.load()

        #expect(loaded.count == 1)
    }

    @Test("an empty cache is empty, not an error")
    func emptyIsNotAnError() async throws {
        let cache = InMemoryFeedCache()

        #expect(try await cache.load().isEmpty)
    }
}

@Suite("Errors")
struct FeedErrorTests {

    @Test("every error describes itself")
    func described() {
        #expect(!describe(FeedError.offline).isEmpty)
    }
}
