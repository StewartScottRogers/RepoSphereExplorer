//
//  DownloadQueue.m
//  A serial download queue with progress callbacks, written in the
//  Foundation idiom: an interface, a class extension holding private
//  state, a protocol, a category, and blocks for the callbacks.
//

#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

typedef NS_ENUM(NSInteger, DownloadState) {
    DownloadStateQueued = 0,
    DownloadStateRunning,
    DownloadStateFinished,
    DownloadStateFailed,
};

typedef void (^DownloadProgressBlock)(double fraction);
typedef void (^DownloadCompletionBlock)(NSData *_Nullable data, NSError *_Nullable error);

@protocol DownloadQueueDelegate <NSObject>
@required
- (void)downloadQueue:(id)queue didFinishURL:(NSURL *)url bytes:(NSUInteger)bytes;
@optional
- (void)downloadQueue:(id)queue didFailURL:(NSURL *)url withError:(NSError *)error;
@end

@interface DownloadTask : NSObject

@property (nonatomic, readonly, copy) NSURL *url;
@property (nonatomic, readonly) DownloadState state;
@property (nonatomic, readonly) NSUInteger bytesReceived;

- (instancetype)initWithURL:(NSURL *)url NS_DESIGNATED_INITIALIZER;
- (instancetype)init NS_UNAVAILABLE;
- (NSString *)describeState;

@end

@interface DownloadQueue : NSObject

@property (nonatomic, weak, nullable) id<DownloadQueueDelegate> delegate;
@property (nonatomic, readonly) NSUInteger pendingCount;
@property (nonatomic, assign) NSTimeInterval timeout;

+ (instancetype)sharedQueue;

- (DownloadTask *)enqueueURL:(NSURL *)url
                    progress:(nullable DownloadProgressBlock)progress
                  completion:(DownloadCompletionBlock)completion;
- (void)cancelAll;
- (NSArray<DownloadTask *> *)tasksInState:(DownloadState)state;

@end

@interface NSURL (RepoSphereSample)
- (BOOL)looksDownloadable;
@end

#pragma mark - Implementation

@interface DownloadTask ()
@property (nonatomic, assign) DownloadState state;
@property (nonatomic, assign) NSUInteger bytesReceived;
@end

@implementation DownloadTask

- (instancetype)initWithURL:(NSURL *)url {
    self = [super init];
    if (self) {
        _url = [url copy];
        _state = DownloadStateQueued;
        _bytesReceived = 0;
    }
    return self;
}

- (NSString *)describeState {
    switch (self.state) {
        case DownloadStateQueued:
            return @"queued";
        case DownloadStateRunning:
            return @"running";
        case DownloadStateFinished:
            return [NSString stringWithFormat:@"finished (%lu bytes)",
                                              (unsigned long)self.bytesReceived];
        case DownloadStateFailed:
            return @"failed";
    }
}

- (NSString *)description {
    return [NSString stringWithFormat:@"<DownloadTask %@ %@>", self.url, [self describeState]];
}

@end

@interface DownloadQueue ()
@property (nonatomic, strong) NSMutableArray<DownloadTask *> *tasks;
@property (nonatomic, strong) NSOperationQueue *operations;
@end

@implementation DownloadQueue

+ (instancetype)sharedQueue {
    static DownloadQueue *shared = nil;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        shared = [[DownloadQueue alloc] init];
    });
    return shared;
}

- (instancetype)init {
    self = [super init];
    if (self) {
        _tasks = [NSMutableArray array];
        _operations = [[NSOperationQueue alloc] init];
        _operations.maxConcurrentOperationCount = 1;
        _timeout = 30.0;
    }
    return self;
}

- (NSUInteger)pendingCount {
    return [self tasksInState:DownloadStateQueued].count;
}

- (DownloadTask *)enqueueURL:(NSURL *)url
                    progress:(nullable DownloadProgressBlock)progress
                  completion:(DownloadCompletionBlock)completion {
    NSParameterAssert(url != nil);
    NSParameterAssert(completion != nil);

    DownloadTask *task = [[DownloadTask alloc] initWithURL:url];
    [self.tasks addObject:task];

    __weak typeof(self) weakSelf = self;
    [self.operations addOperationWithBlock:^{
        typeof(self) strongSelf = weakSelf;
        if (strongSelf == nil) {
            return;
        }
        task.state = DownloadStateRunning;
        if (progress != nil) {
            progress(0.0);
        }

        NSError *error = nil;
        NSData *data = [NSData dataWithContentsOfURL:url
                                             options:NSDataReadingMappedIfSafe
                                               error:&error];
        if (data == nil) {
            task.state = DownloadStateFailed;
            if ([strongSelf.delegate respondsToSelector:@selector(downloadQueue:didFailURL:withError:)]) {
                [strongSelf.delegate downloadQueue:strongSelf didFailURL:url withError:error];
            }
            completion(nil, error);
            return;
        }

        task.bytesReceived = data.length;
        task.state = DownloadStateFinished;
        if (progress != nil) {
            progress(1.0);
        }
        [strongSelf.delegate downloadQueue:strongSelf didFinishURL:url bytes:data.length];
        completion(data, nil);
    }];

    return task;
}

- (void)cancelAll {
    [self.operations cancelAllOperations];
    [self.tasks removeAllObjects];
}

- (NSArray<DownloadTask *> *)tasksInState:(DownloadState)state {
    NSPredicate *predicate = [NSPredicate predicateWithBlock:^BOOL(DownloadTask *task, id bindings) {
        return task.state == state;
    }];
    return [self.tasks filteredArrayUsingPredicate:predicate];
}

@end

@implementation NSURL (RepoSphereSample)

- (BOOL)looksDownloadable {
    NSString *scheme = self.scheme.lowercaseString;
    return [scheme isEqualToString:@"http"] || [scheme isEqualToString:@"https"];
}

@end

NS_ASSUME_NONNULL_END
