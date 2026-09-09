//  DownloadQueue.h
//  A bounded download queue with retries and progress reporting.

#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

/// One download, and everything needed to retry it.
@interface DownloadTask : NSObject

@property (nonatomic, readonly) NSURL *url;
@property (nonatomic, readonly) NSUInteger attempts;
@property (nonatomic, readonly) NSUInteger maxAttempts;

- (instancetype)initWithURL:(NSURL *)url maxAttempts:(NSUInteger)maxAttempts
    NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;

@end

/// Runs at most `maxConcurrent` tasks at a time.
@interface DownloadQueue : NSObject

@property (nonatomic, readonly) NSUInteger maxConcurrent;
@property (nonatomic, readonly) NSUInteger pendingCount;

- (instancetype)initWithMaxConcurrent:(NSUInteger)maxConcurrent NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;

/// Adds a task. Tasks start in the order they were added.
- (void)enqueue:(DownloadTask *)task;

/// Blocks until everything queued has finished or run out of attempts.
- (void)waitUntilFinished;

@end

NS_ASSUME_NONNULL_END
