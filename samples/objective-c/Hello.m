#import <Foundation/Foundation.h>

@interface Greeter : NSObject
- (void)greet:(NSString *)name;
@end

@implementation Greeter
- (void)greet:(NSString *)name {
    NSLog(@"Hello, %@!", name);
}
@end

int main(void) {
    @autoreleasepool {
        Greeter *greeter = [[Greeter alloc] init];
        [greeter greet:@"World"];
    }
    return 0;
}
