import Foundation

/// A feed loader with a cache, an async API and typed failures - the
/// shapes a Swift preview should show: protocols, structs, enums with
/// associated values, actors, extensions and generics.

public enum FeedError: Error, Equatable {
    case network(status: Int)
    case decoding(String)
    case cancelled
    case offline
}

public struct FeedItem: Identifiable, Codable, Hashable {
    public let id: UUID
    public let title: String
    public let author: String
    public let publishedAt: Date
    public let tags: [String]

    public init(id: UUID = UUID(), title: String, author: String, publishedAt: Date, tags: [String] = []) {
        self.id = id
        self.title = title
        self.author = author
        self.publishedAt = publishedAt
        self.tags = tags
    }

    public var summary: String {
        "\(title) - \(author)"
    }
}

public struct Page<Element> {
    public let items: [Element]
    public let nextCursor: String?

    public var isLast: Bool { nextCursor == nil }

    public func map<Other>(_ transform: (Element) throws -> Other) rethrows -> Page<Other> {
        Page<Other>(items: try items.map(transform), nextCursor: nextCursor)
    }
}

public protocol FeedLoading {
    func load(cursor: String?) async throws -> Page<FeedItem>
}

public protocol FeedCaching: AnyObject {
    func cached(for cursor: String?) async -> Page<FeedItem>?
    func store(_ page: Page<FeedItem>, for cursor: String?) async
}

public actor InMemoryFeedCache: FeedCaching {
    private var pages: [String: Page<FeedItem>] = [:]
    private let capacity: Int

    public init(capacity: Int = 16) {
        self.capacity = capacity
    }

    private func key(for cursor: String?) -> String {
        cursor ?? "<first>"
    }

    public func cached(for cursor: String?) async -> Page<FeedItem>? {
        pages[key(for: cursor)]
    }

    public func store(_ page: Page<FeedItem>, for cursor: String?) async {
        if pages.count >= capacity, let victim = pages.keys.first {
            pages.removeValue(forKey: victim)
        }
        pages[key(for: cursor)] = page
    }

    public var count: Int { pages.count }
}

public struct RemoteFeedLoader: FeedLoading {
    private let session: URLSession
    private let endpoint: URL
    private let decoder: JSONDecoder

    public init(endpoint: URL, session: URLSession = .shared) {
        self.endpoint = endpoint
        self.session = session
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        self.decoder = decoder
    }

    public func load(cursor: String?) async throws -> Page<FeedItem> {
        var components = URLComponents(url: endpoint, resolvingAgainstBaseURL: false)
        if let cursor {
            components?.queryItems = [URLQueryItem(name: "cursor", value: cursor)]
        }
        guard let url = components?.url else {
            throw FeedError.decoding("could not build a URL from \(endpoint)")
        }

        let (data, response) = try await session.data(from: url)
        guard let http = response as? HTTPURLResponse else {
            throw FeedError.offline
        }
        guard (200..<300).contains(http.statusCode) else {
            throw FeedError.network(status: http.statusCode)
        }

        do {
            let items = try decoder.decode([FeedItem].self, from: data)
            let next = http.value(forHTTPHeaderField: "X-Next-Cursor")
            return Page(items: items, nextCursor: next)
        } catch {
            throw FeedError.decoding(String(describing: error))
        }
    }
}

public struct CachingFeedLoader: FeedLoading {
    private let inner: FeedLoading
    private let cache: FeedCaching

    public init(inner: FeedLoading, cache: FeedCaching) {
        self.inner = inner
        self.cache = cache
    }

    public func load(cursor: String?) async throws -> Page<FeedItem> {
        if let hit = await cache.cached(for: cursor) {
            return hit
        }
        let page = try await inner.load(cursor: cursor)
        await cache.store(page, for: cursor)
        return page
    }
}

public extension Sequence where Element == FeedItem {
    func byAuthor() -> [String: [FeedItem]] {
        Dictionary(grouping: self, by: \.author)
    }

    func mostRecent(_ count: Int = 5) -> [FeedItem] {
        sorted { $0.publishedAt > $1.publishedAt }.prefix(count).map { $0 }
    }
}

func describe(_ error: FeedError) -> String {
    switch error {
    case .network(let status): return "server said \(status)"
    case .decoding(let detail): return "could not decode: \(detail)"
    case .cancelled: return "cancelled"
    case .offline: return "offline"
    }
}
