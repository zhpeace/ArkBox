//
//  generate.c —— ArkBox Quick Look 生成器
//
//  经典 QLGenerator C 插件 API（CFPlugIn COM vtable）。预览回调里调用同目录的
//  bz-qlhelper 二进制，把压缩包内容渲染成 HTML 后交给 Quick Look 显示；
//  缩略图回调画一张带格式标签的卡片。
//
//  为避免引入依赖 ApplicationServices/CFPlugInCOM 的插件头文件（在 clang 模块
//  模式下会因 IUnknown COM 宏未展开而编译失败），这里只依赖 CoreFoundation /
//  CoreGraphics / QuickLook(QLBase)，并自行声明用到的两个 QL 函数与接口结构体。
//
//  编译：见同目录 build.sh
//

#include <CoreFoundation/CoreFoundation.h>
#include <CoreGraphics/CoreGraphics.h>
#include <spawn.h>
#include <sys/wait.h>
#include <unistd.h>
#include <string.h>
#include <stdlib.h>
#include <limits.h>

// ---- 自行声明 QL 接口（绕过 QLGenerator.h，避免其依赖 ApplicationServices）----
typedef struct __QLPreviewRequest *QLPreviewRequestRef;
typedef struct __QLThumbnailRequest *QLThumbnailRequestRef;
void QLPreviewRequestSetDataRepresentation(QLPreviewRequestRef preview, CFDataRef data,
                                            CFStringRef contentTypeUTI, CFDictionaryRef properties);
void QLThumbnailRequestSetImage(QLThumbnailRequestRef thumbnail, CGImageRef image,
                                CFDictionaryRef properties);

#define noErr 0
typedef int HRESULT;
typedef unsigned int ULONG;
typedef void *REFIID;
typedef void *LPVOID;

typedef struct {
    void *_reserved;
    HRESULT (*QueryInterface)(void *thisPointer, REFIID iid, LPVOID *ppv);
    ULONG (*AddRef)(void *thisPointer);
    ULONG (*Release)(void *thisPointer);
    int (*GenerateThumbnailForURL)(void *thisInterface, QLThumbnailRequestRef thumbnail,
                                   CFURLRef url, CFStringRef contentTypeUTI, CFDictionaryRef options,
                                   CGSize maxSize);
    void (*CancelThumbnailGeneration)(void *thisInterface, QLThumbnailRequestRef thumbnail);
    int (*GeneratePreviewForURL)(void *thisInterface, QLPreviewRequestRef preview, CFURLRef url,
                                 CFStringRef contentTypeUTI, CFDictionaryRef options);
    void (*CancelPreviewGeneration)(void *thisInterface, QLPreviewRequestRef preview);
} QLGeneratorInterfaceStruct;

extern char **environ;

// 找到本生成器可执行文件同目录下的 bz-qlhelper（运行时二者都在 Contents/MacOS）。
static char *find_helper(void) {
    CFBundleRef bundle = CFBundleGetMainBundle();
    if (!bundle) return NULL;
    CFURLRef execURL = CFBundleCopyExecutableURL(bundle);
    if (!execURL) return NULL;
    CFURLRef macosURL = CFURLCreateCopyDeletingLastPathComponent(NULL, execURL);
    CFRelease(execURL);
    CFURLRef helperURL =
        CFURLCreateCopyAppendingPathComponent(NULL, macosURL, CFSTR("bz-qlhelper"), false);
    CFRelease(macosURL);
    if (!helperURL) return NULL;
    CFStringRef path = CFURLCopyFileSystemPath(helperURL, kCFURLPOSIXPathStyle);
    CFRelease(helperURL);
    if (!path) return NULL;
    CFIndex len = CFStringGetLength(path);
    CFIndex bufsz = CFStringGetMaximumSizeForEncoding(len, kCFStringEncodingUTF8) + 1;
    char *out = (char *)malloc((size_t)bufsz);
    if (!out) {
        CFRelease(path);
        return NULL;
    }
    if (!CFStringGetCString(path, out, bufsz, kCFStringEncodingUTF8)) {
        free(out);
        CFRelease(path);
        return NULL;
    }
    CFRelease(path);
    return out;
}

// spawn bz-qlhelper，把 stdout 全部读进 *out（调用方 free），返回 0 成功。
static int run_helper(const char *helper, const char *archive, char **out, size_t *out_len) {
    int out_pipe[2];
    if (pipe(out_pipe) != 0) return -1;

    posix_spawn_file_actions_t fa;
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_adddup2(&fa, out_pipe[1], 1);
    posix_spawn_file_actions_addclose(&fa, out_pipe[0]);
    posix_spawn_file_actions_addclose(&fa, out_pipe[1]);

    const char *argv[] = {helper, archive, NULL};
    pid_t pid = 0;
    int rc = posix_spawn(&pid, helper, &fa, NULL, (char *const *)argv, environ);
    posix_spawn_file_actions_destroy(&fa);
    close(out_pipe[1]);
    if (rc != 0) {
        close(out_pipe[0]);
        return -1;
    }

    size_t cap = 65536;
    size_t used = 0;
    char *buf = (char *)malloc(cap);
    if (!buf) {
        close(out_pipe[0]);
        return -1;
    }
    char tmp[8192];
    ssize_t n;
    while ((n = read(out_pipe[0], tmp, sizeof(tmp))) > 0) {
        if ((size_t)n + used + 1 > cap) {
            while ((size_t)n + used + 1 > cap) cap *= 2;
            char *nb = (char *)realloc(buf, cap);
            if (!nb) {
                free(buf);
                close(out_pipe[0]);
                return -1;
            }
            buf = nb;
        }
        memcpy(buf + used, tmp, (size_t)n);
        used += (size_t)n;
    }
    close(out_pipe[0]);
    int status = 0;
    waitpid(pid, &status, 0);
    buf[used] = '\0';
    *out = buf;
    *out_len = used;
    return 0;
}

int GeneratePreviewForURL(void *thisInterface, QLPreviewRequestRef preview, CFURLRef url,
                          CFStringRef contentTypeUTI, CFDictionaryRef options) {
    (void)thisInterface;
    (void)contentTypeUTI;
    (void)options;

    char archive_path[PATH_MAX];
    if (!CFURLGetFileSystemRepresentation(url, true, (UInt8 *)archive_path, sizeof(archive_path))) {
        return noErr;
    }

    char *helper = find_helper();
    if (!helper) return noErr;

    char *html = NULL;
    size_t html_len = 0;
    int ok = run_helper(helper, archive_path, &html, &html_len);
    free(helper);
    if (ok != 0 || html_len == 0) {
        if (html) free(html);
        return noErr;
    }

    CFDataRef data = CFDataCreate(NULL, (const UInt8 *)html, (CFIndex)html_len);
    free(html);
    if (!data) return noErr;

    CFStringRef htmlUTI = CFSTR("public.html");
    QLPreviewRequestSetDataRepresentation(preview, data, htmlUTI, NULL);
    CFRelease(data);
    return noErr;
}

void CancelPreviewGeneration(void *thisInterface, QLPreviewRequestRef preview) {
    (void)thisInterface;
    (void)preview;
}

// 缩略图：画一张带格式标签的卡片。
static CGImageRef make_thumbnail(CGSize size, const char *label) {
    size_t w = (size_t)(size.width > 0 ? size.width : 512);
    size_t h = (size_t)(size.height > 0 ? size.height : 512);
    size_t stride = w * 4;
    CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
    void *bits = calloc(h, stride);
    if (!bits) {
        CGColorSpaceRelease(cs);
        return NULL;
    }
    CGContextRef ctx = CGBitmapContextCreate(bits, w, h, 8, stride, cs,
                                             kCGImageAlphaPremultipliedFirst);
    if (!ctx) {
        free(bits);
        CGColorSpaceRelease(cs);
        return NULL;
    }

    CGContextSaveGState(ctx);
    CGFloat comps[8] = {0.20, 0.45, 0.85, 1.0, 0.12, 0.28, 0.60, 1.0};
    CGFloat locs[2] = {0.0, 1.0};
    CGGradientRef grad = CGGradientCreateWithColorComponents(cs, comps, locs, 2);
    CGContextDrawLinearGradient(ctx, grad, CGPointMake(0, 0), CGPointMake(0, (CGFloat)h), 0);
    CGGradientRelease(grad);

    CGFloat pad = (CGFloat)w * 0.16f;
    CGRect card = CGRectMake(pad, pad * 1.4f, (CGFloat)w - pad * 2, (CGFloat)h - pad * 2.4f);
    CGContextSetFillColorWithColor(ctx, CGColorCreateGenericRGB(1, 1, 1, 0.92));
    CGFloat r = (CGFloat)w * 0.06f;
    CGContextBeginPath(ctx);
    CGContextMoveToPoint(ctx, card.origin.x + r, card.origin.y);
    CGContextAddArcToPoint(ctx, card.origin.x + card.size.width, card.origin.y,
                           card.origin.x + card.size.width, card.origin.y + card.size.height, r);
    CGContextAddArcToPoint(ctx, card.origin.x + card.size.width, card.origin.y + card.size.height,
                           card.origin.x, card.origin.y + card.size.height, r);
    CGContextAddArcToPoint(ctx, card.origin.x, card.origin.y + card.size.height, card.origin.x,
                           card.origin.y, r);
    CGContextAddArcToPoint(ctx, card.origin.x, card.origin.y,
                           card.origin.x + card.size.width, card.origin.y, r);
    CGContextClosePath(ctx);
    CGContextFillPath(ctx);

    if (label) {
        CGContextSetFillColorWithColor(ctx, CGColorCreateGenericRGB(0.12, 0.30, 0.62, 1.0));
        CGContextSelectFont(ctx, "Helvetica-Bold", (CGFloat)h * 0.28f, kCGEncodingMacRoman);
        CGContextSetTextDrawingMode(ctx, kCGTextFill);
        char buf[16];
        size_t i = 0;
        for (; label[i] && i < sizeof(buf) - 1; i++) {
            char c = label[i];
            buf[i] = (c >= 'a' && c <= 'z') ? (char)(c - 32) : c;
        }
        buf[i] = '\0';
        CGContextShowTextAtPoint(ctx, (CGFloat)w * 0.5f - (CGFloat)strlen(buf) * (CGFloat)h * 0.075f,
                                 (CGFloat)h * 0.42f, buf, strlen(buf));
    }
    CGContextRestoreGState(ctx);

    CGImageRef img = CGBitmapContextCreateImage(ctx);
    CGContextRelease(ctx);
    CGColorSpaceRelease(cs);
    free(bits);
    return img;
}

int GenerateThumbnailForURL(void *thisInterface, QLThumbnailRequestRef thumbnail, CFURLRef url,
                            CFStringRef contentTypeUTI, CFDictionaryRef options, CGSize maxSize) {
    (void)thisInterface;
    (void)options;

    char label[16] = {0};
    if (contentTypeUTI) {
        if (CFStringCompare(contentTypeUTI, CFSTR("public.zip-archive"), 0) == kCFCompareEqualTo)
            strncpy(label, "zip", sizeof(label) - 1);
        else if (CFStringHasSuffix(contentTypeUTI, CFSTR("7-zip-archive")))
            strncpy(label, "7z", sizeof(label) - 1);
        else if (CFStringHasPrefix(contentTypeUTI, CFSTR("public.tar")) ||
                 CFStringHasSuffix(contentTypeUTI, CFSTR("tar-archive")))
            strncpy(label, "tar", sizeof(label) - 1);
        else if (CFStringCompare(contentTypeUTI, CFSTR("public.gzip"), 0) == kCFCompareEqualTo)
            strncpy(label, "gz", sizeof(label) - 1);
    }

    CGSize size = maxSize;
    if (size.width <= 0 || size.height <= 0) size = CGSizeMake(512, 512);
    CGImageRef img = make_thumbnail(size, label[0] ? label : "arc");
    if (img) {
        QLThumbnailRequestSetImage(thumbnail, img, NULL);
        CGImageRelease(img);
    }
    return noErr;
}

void CancelThumbnailGeneration(void *thisInterface, QLThumbnailRequestRef thumbnail) {
    (void)thisInterface;
    (void)thumbnail;
}

// ---- CFPlugin 工厂 ----

static QLGeneratorInterfaceStruct gInterface = {
    NULL, NULL, NULL, NULL,
    GenerateThumbnailForURL,
    CancelThumbnailGeneration,
    GeneratePreviewForURL,
    CancelPreviewGeneration,
};

void *QCGetGeneratorInterface(void) { return &gInterface; }
