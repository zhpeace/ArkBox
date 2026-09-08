// ArkBoxFinderSync.m —— ArkBox 的 Finder Sync Extension（右键独立菜单）。
//
// 行为：Finder 右键文件/文件夹时，菜单里出现独立的「ArkBox」顶级项（带 app 图标）。
// 菜单项按 BetterZip 习惯做上下文感知：
//   - 选中普通文件/文件夹 → 「用 ArkBox 压缩」
//   - 选中压缩包（zip/rar/7z/tar...） → 「用 ArkBox 解压」+「用 ArkBox 打开」
// 「压缩/解压」点击后静默 launch 同 bundle 内的 headless 二进制 bz-cli 干活，
// 不启动 GUI 主程序、不弹窗；「打开」点击交给 ArkBox 主程序进入浏览/编辑界面。
//
// 与旧 NSServices 方案的区别：NSServices 只能塞进「Services」子菜单、且点击时容易
// fallback 启动主 app 弹窗；Finder Sync Extension 是官方机制，菜单独立、点击由本扩展
// 直接拉起 bz-cli，主 app 完全不参与。

#import <Foundation/Foundation.h>
#import <FinderSync/FinderSync.h>
#import <Cocoa/Cocoa.h>

@interface ArkBoxFinderSync : NSObject <FIFinderSync>
@end

@implementation ArkBoxFinderSync

#pragma mark - 菜单

// 仅在「右键文件/文件夹」与「右键边栏项」时返回我们的菜单；工具栏菜单返回 nil。
- (NSMenu *)menuForMenuKind:(FIMenuKind)menuKind {
    if (menuKind == FIMenuKindToolbarItemMenu) {
        return nil;
    }

    // 按选中对象类型决定显示「压缩」还是「解压」（BetterZip 风格）。
    NSArray<NSURL *> *urls = [[FIFinderSyncController defaultController] selectedItemURLs];
    if (urls.count == 0) {
        return nil;  // 右键空白处或没选中文件，不显示菜单。
    }

    BOOL allArchives = [self allItemsAreArchives:urls];

    // 顶级项「ArkBox」带图标 + 子菜单
    NSMenuItem *root = [[NSMenuItem alloc] initWithTitle:@"ArkBox"
                                                  action:nil
                                           keyEquivalent:@""];
    NSImage *icon = [self appIcon];
    if (icon) {
        [icon setSize:NSMakeSize(16, 16)];
        [root setImage:icon];
    }

    NSMenu *sub = [[NSMenu alloc] initWithTitle:@"ArkBox"];

    if (!allArchives) {
        NSMenuItem *compress = [[NSMenuItem alloc] initWithTitle:@"用 ArkBox 压缩"
                                                          action:@selector(arkboxCompress:)
                                                   keyEquivalent:@""];
        [compress setTarget:self];
        [sub addItem:compress];
    }

    if (allArchives) {
        NSMenuItem *extract = [[NSMenuItem alloc] initWithTitle:@"用 ArkBox 解压"
                                                         action:@selector(arkboxExtract:)
                                                  keyEquivalent:@""];
        [extract setTarget:self];
        [sub addItem:extract];

        NSMenuItem *openItem = [[NSMenuItem alloc] initWithTitle:@"用 ArkBox 打开"
                                                          action:@selector(arkboxOpen:)
                                                   keyEquivalent:@""];
        [openItem setTarget:self];
        [sub addItem:openItem];
    }

    if (sub.itemArray.count == 0) {
        return nil;
    }

    [root setSubmenu:sub];

    NSMenu *menu = [[NSMenu alloc] initWithTitle:@"ArkBox"];
    [menu addItem:root];
    return menu;
}

// 判断所有选中项是否都是压缩包（按扩展名）。
- (BOOL)allItemsAreArchives:(NSArray<NSURL *> *)urls {
    // 必须与后端 ArchiveFormat::from_ext 保持一致，否则菜单里出现「解压」但后端认不出。
    NSSet<NSString *> *archiveExts = [NSSet setWithObjects:@"zip", @"zipx", @"jar", @"apk", @"war", @"ear", @"epub",
                                      @"rar", @"7z", @"tar",
                                      @"gz", @"tgz", @"bz2", @"tbz", @"tbz2",
                                      @"xz", @"txz", @"zst", @"tzst",
                                      @"iso", @"cab", @"deb", @"rpm", @"cpio", @"xar", @"br", @"dmg", @"pkg", @"img", nil];
    for (NSURL *u in urls) {
        NSString *ext = u.pathExtension.lowercaseString;
        if (ext.length == 0 || ![archiveExts containsObject:ext]) {
            return NO;
        }
    }
    return YES;
}

#pragma mark - 菜单动作

- (void)arkboxCompress:(id)sender {
    NSArray<NSURL *> *urls = [[FIFinderSyncController defaultController] selectedItemURLs];
    [self runBzCliWithMode:@"compress" urls:urls];
}

- (void)arkboxExtract:(id)sender {
    NSArray<NSURL *> *urls = [[FIFinderSyncController defaultController] selectedItemURLs];
    [self runBzCliWithMode:@"extract" urls:urls];
}

// 「用 ArkBox 打开」：直接启动主程序二进制 arkbox，选中路径作为 argv 传过去，
// 由 lib.rs run() 解析 argv → PendingOpen → emit opened-files（App.vue 进浏览器）。
// 不用 NSWorkspace.openURLs:withApplicationAtURL: —— 那个 API 对已运行的 Tauri app 往往只激活、
// 不投递文档 URL，导致「弹窗但不带文件」。NSTask 直启传 argv 不依赖 AppleEvent 时序，100% 可靠。
- (void)arkboxOpen:(id)sender {
    NSArray<NSURL *> *urls = [[FIFinderSyncController defaultController] selectedItemURLs];
    if (urls.count == 0) {
        return;
    }
    NSString *arkbox = [self arkboxPath];
    if (arkbox == nil || ![NSFileManager.defaultManager fileExistsAtPath:arkbox]) {
        return;
    }
    NSMutableArray<NSString *> *args = [NSMutableArray array];
    for (NSURL *u in urls) {
        if (u.path.length > 0) {
            [args addObject:[u.path stringByStandardizingPath]];
        }
    }
    NSTask *task = [[NSTask alloc] init];
    task.launchPath = arkbox;
    task.arguments = args;
    [task launch];  // 主 app 若已在运行，由 Tauri singleInstance 拦截并转发参数给运行实例
}

#pragma mark - 内部

// launch 同 bundle 内的 bz-cli（绝对路径，避免依赖 PATH）。
- (void)runBzCliWithMode:(NSString *)mode urls:(NSArray<NSURL *> *)urls {
    if (urls.count == 0) {
        return;  // 没有选中任何文件，静默忽略（不弹窗）
    }
    NSString *bzCli = [self bzCliPath];
    if (bzCli == nil || ![NSFileManager.defaultManager fileExistsAtPath:bzCli]) {
        return;
    }

    NSMutableArray<NSString *> *args = [NSMutableArray arrayWithObject:mode];
    for (NSURL *u in urls) {
        if (u.path.length > 0) {
            [args addObject:[u.path stringByStandardizingPath]];
        }
    }

    NSTask *task = [[NSTask alloc] init];
    task.launchPath = bzCli;
    task.arguments = args;
    // 不捕获输出、不等待：让 bz-cli 在后台干完活后自行退出。
    [task launch];
}

// .appex 位于 ArkBox.app/Contents/PlugIns/ArkBoxFinderSync.appex
// → 上两级到 ArkBox.app → Contents/MacOS/bz-cli
- (NSString *)bzCliPath {
    NSString *ext = [[NSBundle mainBundle] bundlePath];
    NSString *contents = [ext stringByDeletingLastPathComponent];  // .../ArkBox.app/Contents
    NSString *app = [contents stringByDeletingLastPathComponent]; // .../ArkBox.app
    return [app stringByAppendingPathComponent:@"Contents/MacOS/bz-cli"];
}

// 同上的路径推导，但指向主程序二进制 arkbox（用于「用 ArkBox 打开」直启传 argv）。
- (NSString *)arkboxPath {
    NSString *ext = [[NSBundle mainBundle] bundlePath];
    NSString *contents = [ext stringByDeletingLastPathComponent];
    NSString *app = [contents stringByDeletingLastPathComponent];
    return [app stringByAppendingPathComponent:@"Contents/MacOS/arkbox"];
}

// 用宿主 app 的图标作菜单项图标（extension 自身 bundle 没有好图标）。
- (NSImage *)appIcon {
    NSString *ext = [[NSBundle mainBundle] bundlePath];
    NSString *contents = [ext stringByDeletingLastPathComponent];
    NSString *app = [contents stringByDeletingLastPathComponent];
    if (app.length > 0) {
        return [[NSWorkspace sharedWorkspace] iconForFile:app];
    }
    return nil;
}

@end
