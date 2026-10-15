#import <Foundation/Foundation.h>
#import <WebKit/WebKit.h>
#import <objc/message.h>
#import <objc/runtime.h>
#include <dlfcn.h>
#include <limits.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

// Declare the init family so ARC preserves its consumed-self / retained-result contract.
@interface NSObject (TabbyRSStorageDirectory)
- (instancetype)initWithDirectory:(NSURL *)directory;
@end

@interface WKWebsiteDataStore (TabbyRSStorageConfiguration)
- (instancetype)_initWithConfiguration:(id)configuration __attribute__((objc_method_family(init)));
@end

static WKWebsiteDataStore *gOwnedWebsiteDataStore;
static NSURL *gOwnedDirectoryURL;
static NSString *gOwnedCanonicalDirectory;
static BOOL gHookInstalled;

static void set_error(char *error, size_t capacity, NSString *message) {
    if (error == NULL || capacity == 0) return;
    const char *text = message.UTF8String;
    if (text == NULL || text[0] == '\0') text = "portable WebView storage installation failed";
    snprintf(error, capacity, "%s", text);
}

static BOOL object_type(const char *type) {
    return type != NULL && type[0] == '@';
}

static BOOL method_matches(Method method, NSUInteger argumentCount, BOOL classReceiver) {
    if (method == NULL || method_getNumberOfArguments(method) != argumentCount) return NO;

    char *returnType = method_copyReturnType(method);
    BOOL valid = object_type(returnType);
    free(returnType);
    if (!valid) return NO;

    char *selfType = method_copyArgumentType(method, 0);
    valid = selfType != NULL
        && (classReceiver ? (selfType[0] == '#' || selfType[0] == '@')
                          : selfType[0] == '@');
    free(selfType);
    if (!valid) return NO;

    char *selectorType = method_copyArgumentType(method, 1);
    valid = selectorType != NULL && selectorType[0] == ':';
    free(selectorType);
    if (!valid) return NO;

    for (NSUInteger index = 2; index < argumentCount; index++) {
        char *argumentType = method_copyArgumentType(method, (unsigned int)index);
        valid = object_type(argumentType);
        free(argumentType);
        if (!valid) return NO;
    }
    return YES;
}

static BOOL void_object_setter_matches(Method method) {
    if (method == NULL || method_getNumberOfArguments(method) != 3) return NO;

    char *returnType = method_copyReturnType(method);
    BOOL valid = returnType != NULL && returnType[0] == 'v';
    free(returnType);
    if (!valid) return NO;

    char *selfType = method_copyArgumentType(method, 0);
    valid = selfType != NULL && selfType[0] == '@';
    free(selfType);
    if (!valid) return NO;

    char *selectorType = method_copyArgumentType(method, 1);
    valid = selectorType != NULL && selectorType[0] == ':';
    free(selectorType);
    if (!valid) return NO;

    char *argumentType = method_copyArgumentType(method, 2);
    valid = argumentType != NULL && argumentType[0] == '@';
    free(argumentType);
    return valid;
}

static Method direct_class_method(Class metaclass, SEL selector) {
    unsigned int count = 0;
    Method *methods = class_copyMethodList(metaclass, &count);
    Method found = NULL;
    for (unsigned int index = 0; index < count; index++) {
        if (method_getName(methods[index]) == selector) {
            found = methods[index];
            break;
        }
    }
    free(methods);
    return found;
}

static BOOL imp_belongs_to_system_webkit(IMP implementation) {
    Dl_info info;
    if (implementation == NULL) return NO;
    if (dladdr((void *)implementation, &info) == 0) return NO;
    const char *classImage = class_getImageName(WKWebsiteDataStore.class);
    return classImage != NULL && info.dli_fname != NULL
        && strcmp(info.dli_fname, classImage) == 0;
}

static Class configuration_class(void) {
    return NSClassFromString(@"_WKWebsiteDataStoreConfiguration");
}

static SEL directory_initializer_selector(void) {
    return NSSelectorFromString(@"initWithDirectory:");
}

static WKWebsiteDataStore *create_store(id configuration, NSString **failure) {
    SEL initializer = NSSelectorFromString(@"_initWithConfiguration:");
    Method method = class_getInstanceMethod(WKWebsiteDataStore.class, initializer);
    if (!method_matches(method, 3, NO)) {
        *failure = @"the WebKit storage initializer has an incompatible ABI";
        return nil;
    }
    WKWebsiteDataStore *store = nil;
    @try {
        // This initializer predates the macOS 27 class factory.
        store = [[WKWebsiteDataStore alloc] _initWithConfiguration:configuration];
    } @catch (NSException *exception) {
        *failure = [NSString stringWithFormat:@"directory-backed store creation raised an exception: %@",
                    exception.reason ?: exception.name ?: @"unknown"];
        return nil;
    }
    if (store == nil) *failure = @"directory-backed store creation returned nil";
    return store;
}

static __attribute__((unused)) BOOL new_directory_api_available(Class configurationClass) {
    if (configurationClass == Nil) return NO;
    return class_getInstanceMethod(configurationClass, directory_initializer_selector()) != NULL;
}

static __attribute__((unused)) WKWebsiteDataStore *create_store_with_directory(Class configurationClass, NSString **failure) {
    SEL initializer = directory_initializer_selector();
    Method initializerMethod = class_getInstanceMethod(configurationClass, initializer);
    if (![configurationClass instancesRespondToSelector:initializer]
        || !method_matches(initializerMethod, 3, NO)) {
        *failure = @"the private directory WebKit storage API has an incompatible ABI";
        return nil;
    }


    id configuration = nil;
    @try {
        configuration = [[configurationClass alloc] initWithDirectory:gOwnedDirectoryURL];
    } @catch (NSException *exception) {
        *failure = [NSString stringWithFormat:@"private directory configuration raised an exception: %@",
                    exception.reason ?: exception.name ?: @"unknown"];
        return nil;
    }
    if (configuration == nil) {
        *failure = @"private directory configuration initialization returned nil";
        return nil;
    }

    return create_store(configuration, failure);
}

typedef struct {
    const char *selectorName;
    const char *directoryName;
    BOOL required;
    BOOL isFile;
} legacy_setter;

static WKWebsiteDataStore *create_store_with_legacy_setters(Class configurationClass, NSString **failure) {
    static const legacy_setter setters[] = {
        { "_setWebStorageDirectory:", "LocalStorage", YES, NO },
        { "_setIndexedDBDatabaseDirectory:", "IndexedDB", YES, NO },
        { "_setWebSQLDatabaseDirectory:", "WebSQL", YES, NO },
        { "_setCookieStorageFile:", "Cookies/Cookies.binarycookies", YES, YES },
        { "_setResourceLoadStatisticsDirectory:", "ResourceLoadStatistics", NO, NO },
        { "_setCacheStorageDirectory:", "CacheStorage", NO, NO },
        { "_setServiceWorkerRegistrationDirectory:", "ServiceWorkers", NO, NO },
        { "setNetworkCacheDirectory:", "NetworkCache", NO, NO },
        { "setDeviceIdHashSaltsStorageDirectory:", "DeviceIdHashSalts", NO, NO },
        { "setMediaCacheDirectory:", "MediaCache", NO, NO },
        { "setMediaKeysStorageDirectory:", "MediaKeys", NO, NO },
        { "setHSTSStorageDirectory:", "HSTS", NO, NO },
        { "setAlternativeServicesStorageDirectory:", "AlternativeServices", NO, NO },
        { "setGeneralStorageDirectory:", "Origins", NO, NO },
    };
    const NSUInteger setterCount = sizeof(setters) / sizeof(setters[0]);

    SEL initializer = @selector(init);
    Method initializerMethod = class_getInstanceMethod(configurationClass, initializer);
    if (configurationClass == Nil
        || ![configurationClass instancesRespondToSelector:initializer]
        || !method_matches(initializerMethod, 2, NO)) {
        *failure = @"the legacy WebKit storage configuration API has an incompatible ABI";
        return nil;
    }

    id configuration = nil;
    @try {
        configuration = [[configurationClass alloc] init];
    } @catch (NSException *exception) {
        *failure = [NSString stringWithFormat:@"legacy configuration initialization raised an exception: %@",
                    exception.reason ?: exception.name ?: @"unknown"];
        return nil;
    }
    if (configuration == nil) {
        *failure = @"legacy configuration initialization returned nil";
        return nil;
    }

    typedef void (*DirectorySetterIMP)(id, SEL, NSURL *);
    for (NSUInteger index = 0; index < setterCount; index++) {
        NSString *selectorName = [NSString stringWithUTF8String:setters[index].selectorName];
        SEL selector = NSSelectorFromString(selectorName);
        Method method = class_getInstanceMethod(configurationClass, selector);
        if (method == NULL) {
            if (setters[index].required) {
                *failure = [NSString stringWithFormat:@"the required legacy storage setter %s is unavailable",
                            setters[index].selectorName];
                return nil;
            }
            continue;
        }
        if (!void_object_setter_matches(method)) {
            *failure = [NSString stringWithFormat:@"the legacy storage setter %s has an incompatible ABI",
                        setters[index].selectorName];
            return nil;
        }

        NSString *target = [gOwnedDirectoryURL.path stringByAppendingPathComponent:
            [NSString stringWithUTF8String:setters[index].directoryName]];
        NSString *directoryPath = setters[index].isFile ? target.stringByDeletingLastPathComponent : target;
        NSError *directoryError = nil;
        if (![[NSFileManager defaultManager] createDirectoryAtPath:directoryPath
                                       withIntermediateDirectories:YES
                                                        attributes:nil
                                                             error:&directoryError]) {
            *failure = [NSString stringWithFormat:@"unable to create the owned legacy storage directory %@: %@",
                        directoryPath, directoryError.localizedDescription ?: @"unknown error"];
            return nil;
        }
        NSURL *targetURL = [NSURL fileURLWithPath:target isDirectory:!setters[index].isFile];
        if (targetURL == nil || !targetURL.isFileURL) {
            *failure = @"unable to build a file URL for a legacy storage location";
            return nil;
        }
        @try {
            ((DirectorySetterIMP)objc_msgSend)(configuration, selector, targetURL);
        } @catch (NSException *exception) {
            *failure = [NSString stringWithFormat:@"the legacy storage setter %s raised an exception: %@",
                        setters[index].selectorName, exception.reason ?: exception.name ?: @"unknown"];
            return nil;
        }
    }

    return create_store(configuration, failure);
}

static id owned_default_data_store(id receiver, SEL selector) {
    (void)receiver;
    (void)selector;
    @synchronized (WKWebsiteDataStore.class) {
        return gOwnedWebsiteDataStore;
    }
}

int tabby_rs_install_portable_webview_storage(const char *directory, char *error, size_t capacity) {
    @autoreleasepool {
        if (directory == NULL) {
            set_error(error, capacity, @"portable WebView storage directory is null");
            return 0;
        }
        if (![NSThread isMainThread]) {
            set_error(error, capacity, @"portable WebView storage must be installed on the main thread");
            return 0;
        }

        NSString *input = [[NSFileManager defaultManager]
            stringWithFileSystemRepresentation:directory length:strlen(directory)];
        if (input.length == 0) {
            set_error(error, capacity, @"portable WebView storage directory is empty");
            return 0;
        }
        if (!input.isAbsolutePath) {
            set_error(error, capacity, @"portable WebView storage directory must be an absolute path");
            return 0;
        }

        char resolved[PATH_MAX];
        if (realpath(input.fileSystemRepresentation, resolved) == NULL) {
            set_error(error, capacity, @"portable WebView storage directory cannot be resolved on the file system");
            return 0;
        }
        NSString *canonical = [[NSFileManager defaultManager]
            stringWithFileSystemRepresentation:resolved length:strlen(resolved)];
        if (canonical == nil || canonical.length == 0) {
            set_error(error, capacity, @"portable WebView storage directory is not a valid file-system path");
            return 0;
        }
        BOOL isDirectory = NO;
        if (![[NSFileManager defaultManager] fileExistsAtPath:canonical isDirectory:&isDirectory]
            || !isDirectory) {
            set_error(error, capacity, @"portable WebView storage directory is not an existing directory");
            return 0;
        }
        if (access(resolved, W_OK) != 0) {
            set_error(error, capacity, @"portable WebView storage directory is not writable");
            return 0;
        }

        @synchronized (WKWebsiteDataStore.class) {
            Class storeMetaclass = object_getClass(WKWebsiteDataStore.class);
            if (gHookInstalled) {
                Method currentOwn = direct_class_method(storeMetaclass, @selector(defaultDataStore));
                if (currentOwn == NULL
                    || method_getImplementation(currentOwn) != (IMP)owned_default_data_store) {
                    set_error(error, capacity,
                        @"the portable WebView storage hook was replaced by another implementation");
                    return 0;
                }
                if (![canonical isEqualToString:gOwnedCanonicalDirectory]) {
                    set_error(error, capacity, [NSString stringWithFormat:
                        @"portable WebView storage is already installed for a different directory: %@",
                        gOwnedCanonicalDirectory]);
                    return 0;
                }
                return 1;
            }

            Method defaultMethod = class_getClassMethod(WKWebsiteDataStore.class, @selector(defaultDataStore));
            if (!method_matches(defaultMethod, 2, YES)) {
                set_error(error, capacity, @"WKWebsiteDataStore defaultDataStore has an incompatible ABI");
                return 0;
            }
            if (!imp_belongs_to_system_webkit(method_getImplementation(defaultMethod))) {
                set_error(error, capacity, @"refusing to replace a non-WebKit defaultDataStore implementation");
                return 0;
            }

            NSURL *directoryURL = [NSURL fileURLWithPath:canonical isDirectory:YES];
            if (directoryURL == nil || !directoryURL.isFileURL) {
                set_error(error, capacity, @"unable to build a file URL for the portable WebView storage directory");
                return 0;
            }
            gOwnedDirectoryURL = directoryURL;

            NSString *creationFailure = nil;
            Class configurationClass = configuration_class();
            WKWebsiteDataStore *store = nil;
#ifdef TABBY_RS_TEST_LEGACY_STORAGE
            store = create_store_with_legacy_setters(configurationClass, &creationFailure);
#else
            if (new_directory_api_available(configurationClass)) {
                store = create_store_with_directory(configurationClass, &creationFailure);
            } else {
                store = create_store_with_legacy_setters(configurationClass, &creationFailure);
            }
#endif
            if (store == nil) {
                gOwnedDirectoryURL = nil;
                set_error(error, capacity, creationFailure ?: @"unable to create the portable WebView storage store");
                return 0;
            }

            if (!class_addMethod(storeMetaclass, @selector(defaultDataStore),
                    (IMP)owned_default_data_store, method_getTypeEncoding(defaultMethod))) {
                Method ownMethod = direct_class_method(storeMetaclass, @selector(defaultDataStore));
                if (!method_matches(ownMethod, 2, YES)) {
                    gOwnedDirectoryURL = nil;
                    set_error(error, capacity, @"cannot install a class-local defaultDataStore hook");
                    return 0;
                }
                method_setImplementation(ownMethod, (IMP)owned_default_data_store);
            }

            gOwnedWebsiteDataStore = store;
            gOwnedCanonicalDirectory = [canonical copy];
            gHookInstalled = YES;
            return 1;
        }
    }
}
