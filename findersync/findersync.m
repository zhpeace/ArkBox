#import <Foundation/Foundation.h>
#import <AppKit/AppKit.h>
#import <FinderSync/FinderSync.h>

// 主 app 的 bundle identifier（与 tauri.conf.json 的 identifier 一致）
static NSString *const kHostBundleID = @"com.arkbox.desktop";

// Finder Sync Extension 的 principal class。
// 注意：Apple 的 FinderSync.framework 已声明了名为 FIFinderSync 的类，
// 因此这里用 FinderSync 作为实现类名，避免重定义冲突。
@interface FinderSync : NSObject <FIFinderSync>
@end

@implementation FinderSync

- (instancetype)init {
    self = [super init];
    if (self) {
        // 声明要观察的目录：用户家目录 + 外接卷，这样常见位置的右键菜单都会出现。
        // 根目录 / 也可加，但观察全盘会触发大量 beginObserving 回调，这里不铺太广。
        FIFinderSyncController *ctrl = [FIFinderSyncController defaultController];
        NSMutableSet<NSURL *> *dirs = [NSMutableSet set];
        [dirs addObject:[NSURL fileURLWithPath:NSHomeDirectory()]];
        [dirs addObject:[NSURL fileURLWithPath:@"/Volumes"]];
        ctrl.directoryURLs = dirs;
    }
    return self;
}

// 观察目录回调（协议可选，留空即可）
- (void)beginObservingDirectoryAtURL:(NSURL *)url { (void)url; }
- (void)endObservingDirectoryAtURL:(NSURL *)url { (void)url; }

// 右键菜单：文件/文件夹/窗口背景都加上两项
- (NSMenu *)menuForMenuKind:(FIMenuKind)menu {
    (void)menu;
    NSMenu *m = [[NSMenu alloc] initWithTitle:@""];

    NSMenuItem *openItem = [[NSMenuItem alloc]
        initWithTitle:@"用 ArkBox 打开"
               action:@selector(bzOpen:)
        keyEquivalent:@""];
    NSMenuItem *compressItem = [[NSMenuItem alloc]
        initWithTitle:@"压缩所选 (ArkBox)"
               action:@selector(bzCompress:)
        keyEquivalent:@""];

    [m addItem:openItem];
    [m addItem:compressItem];
    return m;
}

// 把选中文件甩给主 app：复用了主 app 已有的 RunEvent::Opened -> handleDrop
// 分发逻辑（单压缩包->浏览，多文件/目录->预填压缩）。
- (void)launchHostWithSelectedURLs {
    NSArray<NSURL *> *urls = [[FIFinderSyncController defaultController] selectedItemURLs];
    if (urls.count == 0) return;
    [[NSWorkspace sharedWorkspace] openURLs:urls
                     withAppBundleIdentifier:kHostBundleID
                                     options:NSWorkspaceLaunchDefault
              additionalEventParamDescriptor:nil
                           launchIdentifiers:nil];
}

- (void)bzOpen:(id)sender { (void)sender; [self launchHostWithSelectedURLs]; }
- (void)bzCompress:(id)sender { (void)sender; [self launchHostWithSelectedURLs]; }

@end
