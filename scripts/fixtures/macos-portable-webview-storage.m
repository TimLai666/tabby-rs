#import <Cocoa/Cocoa.h>
#import <WebKit/WebKit.h>
#import <objc/message.h>
#import <objc/runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>
#include <unistd.h>
#include <pthread.h>
#include <dispatch/dispatch.h>
extern int tabby_rs_install_portable_webview_storage(const char *, char *, size_t);

static WKWebsiteDataStore *conflicting_store(id receiver, SEL selector) {
    (void)receiver; (void)selector;
    return nil;
}
static NSDate *gRunDeadline;

typedef struct {
    const char *path;
    dispatch_semaphore_t finished;
    BOOL rejected;
} off_main_check;

static void *run_off_main_check(void *opaque) {
    @autoreleasepool {
        off_main_check *check = opaque;
        BOOL reallyOffMain = !NSThread.isMainThread;
        char error[256] = {0};
        int result = tabby_rs_install_portable_webview_storage(check->path, error, sizeof(error));
        check->rejected = reallyOffMain && result == 0 && error[0] != '\0';
        dispatch_semaphore_signal(check->finished);
    }
    return NULL;
}

static BOOL guard_check(BOOL result, const char *label) {
    if (!result) fprintf(stderr, "guards failed: %s\n", label);
    return result;
}

@interface StorageNavigationObserver : NSObject <WKNavigationDelegate>
@property(nonatomic) BOOL finished;
@property(nonatomic, strong) NSError *failure;
@end

@implementation StorageNavigationObserver
- (void)webView:(WKWebView *)webView didFinishNavigation:(WKNavigation *)navigation {
    (void)webView; (void)navigation;
    self.finished = YES;
}
- (void)webView:(WKWebView *)webView didFailNavigation:(WKNavigation *)navigation
      withError:(NSError *)error {
    (void)webView; (void)navigation;
    self.failure = error;
}
- (void)webView:(WKWebView *)webView didFailProvisionalNavigation:(WKNavigation *)navigation
      withError:(NSError *)error {
    (void)webView; (void)navigation;
    self.failure = error;
}
@end

static BOOL install_rejected(const char *path, IMP expected, BOOL checkImp, BOOL checkErrorNUL) {
    char error[256]; memset(error, 0x7f, sizeof(error));
    int result = tabby_rs_install_portable_webview_storage(path, error, sizeof(error));
    if (result != 0 || error[0] == '\0') return NO;
    if (checkErrorNUL && memchr(error, '\0', sizeof(error)) == NULL) return NO;
    if (checkImp) {
        Method m = class_getClassMethod(WKWebsiteDataStore.class, @selector(defaultDataStore));
        if (m == NULL || method_getImplementation(m) != expected) return NO;
    }
    return YES;
}

static id wait_js(WKWebView *view, NSString *script, BOOL *ok) {
    __block id result = nil; __block BOOL done = NO; __block NSError *failure = nil;
    [view evaluateJavaScript:script completionHandler:^(id value, NSError *error) {
        result = value; failure = error; done = YES;
    }];
    NSDate *limit = gRunDeadline ?: [NSDate dateWithTimeIntervalSinceNow:10.0];
    while (!done && [limit timeIntervalSinceNow] > 0) {
        @autoreleasepool { [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
            beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.02]]; }
    }
    *ok = done && failure == nil;
    if (failure != nil)
        fprintf(stderr, "JavaScript evaluation failed (%s): %s\n", script.UTF8String,
            failure.localizedDescription.UTF8String ?: "unknown error");
    else if (!done)
        fprintf(stderr, "JavaScript evaluation timed out (%s)\n", script.UTF8String);
    return result;
}

static BOOL load_view(WKWebView *view) {
    StorageNavigationObserver *observer = [StorageNavigationObserver new];
    view.navigationDelegate = observer;
    [view loadHTMLString:@"<!doctype html><meta charset=utf-8><title>storage</title>"
                  baseURL:[NSURL URLWithString:@"https://tabby-rs-storage.invalid/"]];
    NSDate *limit = gRunDeadline ?: [NSDate dateWithTimeIntervalSinceNow:10.0];
    while (!observer.finished && observer.failure == nil && [limit timeIntervalSinceNow] > 0) {
        @autoreleasepool { [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
            beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.02]]; }
    }
    if (observer.failure != nil)
        fprintf(stderr, "WKWebView navigation failed: %s\n",
            observer.failure.localizedDescription.UTF8String ?: "unknown error");
    else if (!observer.finished)
        fprintf(stderr, "WKWebView navigation timed out before didFinish\n");
    return observer.finished && observer.failure == nil;
}

static NSString *js_literal(NSString *s) {
    NSData *d = [NSJSONSerialization dataWithJSONObject:@[s] options:NSJSONWritingFragmentsAllowed error:nil];
    NSString *array = [[NSString alloc] initWithData:d encoding:NSUTF8StringEncoding];
    return [array substringWithRange:NSMakeRange(1, array.length - 2)];
}

static void emit(NSString *name, BOOL passed, id before, id after, id secondary, BOOL storageCase) {
    NSMutableDictionary *out = [@{@"case":name, @"passed":@(passed)} mutableCopy];
    if (storageCase) {
        out[@"before"] = before ?: [NSNull null]; out[@"after"] = after ?: [NSNull null];
        out[@"secondary"] = secondary ?: [NSNull null];
    }
    NSData *data = [NSJSONSerialization dataWithJSONObject:out options:NSJSONWritingFragmentsAllowed error:nil];
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    printf("TABBY_STORAGE_NATIVE_EVIDENCE %s\n", json.UTF8String ?: "{}");
}

// Test entry: case, existing owned directory, key, value.
// Emit exactly one TABBY_STORAGE_NATIVE_EVIDENCE JSON record and return zero
// only after checking the requested behavior. No shared/real user storage.
static int run_case(NSString *name, NSString *directory, NSString *key, NSString *value) {
    gRunDeadline = [NSDate dateWithTimeIntervalSinceNow:10.0];
    Method method = class_getClassMethod(WKWebsiteDataStore.class, @selector(defaultDataStore));
    if (method == NULL) { emit(name, NO, nil, nil, nil, NO); return 1; }
    IMP original = method_getImplementation(method);
    BOOL passed = YES;
    if ([name isEqual:@"guards"]) {
        char missing[PATH_MAX]; snprintf(missing, sizeof(missing), "%s/does-not-exist", directory.fileSystemRepresentation);
        char filePath[PATH_MAX]; snprintf(filePath, sizeof(filePath), "%s/guard-file", directory.fileSystemRepresentation);
        FILE *f = fopen(filePath, "w"); if (f == NULL) passed = NO; else { fputs("x", f); fclose(f); }
        passed = guard_check(passed, "create owned file") && passed;
        passed = guard_check(install_rejected(NULL, original, YES, YES), "NULL path") && passed;
        passed = guard_check(install_rejected("", original, YES, YES), "empty path") && passed;
        passed = guard_check(install_rejected("relative-storage-path", original, YES, YES), "relative path") && passed;
        passed = guard_check(install_rejected(missing, original, YES, YES), "nonexisting path") && passed;
        passed = guard_check(install_rejected(filePath, original, YES, YES), "file path") && passed;
        off_main_check *check = calloc(1, sizeof(*check));
        if (check != NULL) {
            check->path = directory.fileSystemRepresentation;
            check->finished = dispatch_semaphore_create(0);
        }
        pthread_t worker;
        int threadResult = check == NULL ? -1 : pthread_create(&worker, NULL, run_off_main_check, check);
        BOOL threadStarted = threadResult == 0;
        BOOL offMainRejected = NO;
        if (threadStarted) {
            long waited = dispatch_semaphore_wait(check->finished,
                dispatch_time(DISPATCH_TIME_NOW, (int64_t)(10 * NSEC_PER_SEC)));
            if (waited == 0) {
                pthread_join(worker, NULL);
                offMainRejected = check->rejected;
                free(check);
            } else {
                pthread_detach(worker);
            }
        }
        if (check != NULL && !threadStarted) free(check);
        passed = guard_check(threadStarted, "create off-main pthread") && passed;
        passed = guard_check(offMainRejected, "off-main rejection and error") && passed;
        method = class_getClassMethod(WKWebsiteDataStore.class, @selector(defaultDataStore));
        passed = guard_check(method != NULL && method_getImplementation(method) == original,
            "defaultDataStore IMP unchanged") && passed;
    } else if ([name isEqual:@"conflicting-hook"]) {
        method_setImplementation(method, (IMP)conflicting_store);
        char error[256] = {0};
        BOOL rejected = tabby_rs_install_portable_webview_storage(directory.fileSystemRepresentation,
            error, sizeof(error)) == 0 && error[0] != '\0';
        Method now = class_getClassMethod(WKWebsiteDataStore.class, @selector(defaultDataStore));
        BOOL callable = ((WKWebsiteDataStore *(*)(id, SEL))objc_msgSend)(WKWebsiteDataStore.class,
            @selector(defaultDataStore)) == nil;
        passed = rejected && now != NULL && method_getImplementation(now) == (IMP)conflicting_store && callable;
    } else if ([name isEqual:@"repeat-install"]) {
        char error[256] = {0};
        BOOL first = tabby_rs_install_portable_webview_storage(directory.fileSystemRepresentation,error,sizeof(error)) != 0;
        WKWebsiteDataStore *store = first ? WKWebsiteDataStore.defaultDataStore : nil;
        BOOL same = first && tabby_rs_install_portable_webview_storage(directory.fileSystemRepresentation,error,sizeof(error)) != 0;
        WKWebsiteDataStore *again = same ? WKWebsiteDataStore.defaultDataStore : nil;
        BOOL persistent = store != nil && store.isPersistent;
        NSString *sibling = [directory stringByAppendingPathComponent:@"other-owned-store"];
        NSError *createError = nil;
        BOOL siblingMade = [[NSFileManager defaultManager] createDirectoryAtPath:sibling
            withIntermediateDirectories:NO attributes:nil error:&createError];
        BOOL differentRejected = siblingMade && install_rejected(sibling.fileSystemRepresentation,
            method_getImplementation(class_getClassMethod(WKWebsiteDataStore.class,@selector(defaultDataStore))),YES,YES);
        passed = first && same && store != nil && store == again && persistent && differentRejected
            && WKWebsiteDataStore.defaultDataStore == store;
    } else if ([name isEqual:@"write"] || [name isEqual:@"read"]) {
        char error[256] = {0};
        BOOL installed = tabby_rs_install_portable_webview_storage(directory.fileSystemRepresentation,error,sizeof(error)) != 0;
        if (!installed) {
            fprintf(stderr, "%s failed: installer rejected owned directory: %s\n", name.UTF8String, error);
            emit(name, NO, nil, nil, nil, YES);
            return 1;
        }
        WKWebsiteDataStore *store = WKWebsiteDataStore.defaultDataStore;
        if (store == nil) {
            fprintf(stderr, "%s failed: installed hook returned nil store\n", name.UTF8String);
            emit(name, NO, nil, nil, nil, YES);
            return 1;
        }
        WKWebViewConfiguration *config = [WKWebViewConfiguration new]; config.websiteDataStore = store;
        WKWebView *primary = [[WKWebView alloc] initWithFrame:NSZeroRect configuration:config];
        BOOL loaded = installed && store != nil && store.isPersistent && load_view(primary);
        NSString *literal = js_literal(key);
        BOOL ok = NO; id before = loaded ? wait_js(primary,
            [NSString stringWithFormat:@"localStorage.getItem(%@)",literal],&ok) : nil;
        BOOL beforeOK = loaded && ok;
        BOOL afterOK = NO; id after = nil;
        if ([name isEqual:@"write"]) {
            BOOL setOK = NO;
            (void)wait_js(primary,[NSString stringWithFormat:@"localStorage.setItem(%@,%@); localStorage.getItem(%@)",
                literal,js_literal(value),literal],&setOK);
            NSDate *flush = [NSDate dateWithTimeIntervalSinceNow:1.5];
            while ([flush timeIntervalSinceNow] > 0)
                [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
            after = wait_js(primary,[NSString stringWithFormat:@"localStorage.getItem(%@)",literal],&afterOK);
            afterOK = afterOK && setOK;
        } else {
            after = wait_js(primary,[NSString stringWithFormat:@"localStorage.getItem(%@)",literal],&afterOK);
        }
        WKWebViewConfiguration *config2 = [WKWebViewConfiguration new]; config2.websiteDataStore = store;
        WKWebView *secondaryView = [[WKWebView alloc] initWithFrame:NSZeroRect configuration:config2];
        BOOL secondLoaded = load_view(secondaryView), secondaryOK = NO;
        id secondary = secondLoaded ? wait_js(secondaryView,[NSString stringWithFormat:@"localStorage.getItem(%@)",literal],&secondaryOK) : nil;
        if ([name isEqual:@"write"])
            passed = loaded && beforeOK && before == [NSNull null] && afterOK && [after isEqual:value]
                && secondLoaded && secondaryOK && [secondary isEqual:value];
        else
            passed = loaded && beforeOK && afterOK && ((before == [NSNull null] && after == [NSNull null]) || [before isEqual:after])
                && secondLoaded && secondaryOK && ((secondary == [NSNull null] && after == [NSNull null]) || [secondary isEqual:after]);
        emit(name, passed, before, after, secondary, YES);
        return passed ? 0 : 1;
    } else {
        emit(name, NO, nil, nil, nil, NO); return 1;
    }
    emit(name, passed, nil, nil, nil, NO);
    return passed ? 0 : 1;
}

int main(int argc, const char **argv) {
    @autoreleasepool {
        if (argc != 5) return 64;
        NSString *bundleID = NSBundle.mainBundle.bundleIdentifier;
        if (![bundleID hasPrefix:@"dev.tabbyrs.storage-test."]) return 65;
        return run_case(@(argv[1]), @(argv[2]), @(argv[3]), @(argv[4]));
    }
}
