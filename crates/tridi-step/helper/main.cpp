// tridi-step-helper PATH [--coarse] [--only ID,ID,...]: tessellates a STEP
// file through occt.cpp and writes the wire tridi-step's parent.rs reads,
// on stdout: a u32 count and that many u64 part ids; then per part its u64
// id, a u32 byte length and that many bytes of f32 (position, normal, color
// per vertex), all little-endian. Exit 0, or 1 not STEP, 2 no solid, 3 the
// parent stopped reading, 64 bad arguments.

#include <sys/prctl.h>
#include <unistd.h>

#include <csignal>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <vector>

using IdsFn = int (*)(void *, const uint64_t *, size_t);
using PartFn = int (*)(void *, uint64_t, const float *, size_t);
extern "C" int tridi_step_tessellate(const char *path, int coarse, const uint64_t *only, size_t n_only, void *ctx,
                                     IdsFn ids, PartFn part);

namespace {

// bad arguments (sysexits.h's EX_USAGE)
constexpr int USAGE = 64;
// the parent stopped reading (occt.cpp's WRITE)
constexpr int WRITE = 3;

std::mutex out;

// the frames' own stream, on what was fd 1: fd 1 itself goes to stderr, so
// whatever OpenCASCADE's messenger prints to std::cout can't break a frame
std::FILE *wire = nullptr;

// whole frames only: the parts come from several threads
bool put(const void *p, size_t n) { return std::fwrite(p, 1, n, wire) == n; }

// a pipe is fully buffered: each frame is flushed as it's done
int on_ids(void *, const uint64_t *ids, size_t n) {
    std::lock_guard<std::mutex> lock(out);
    auto count = static_cast<uint32_t>(n);
    bool ok = put(&count, 4) && (n == 0 || put(ids, n * 8));
    return ok && std::fflush(wire) == 0 ? 0 : 1;
}

int on_part(void *, uint64_t id, const float *v, size_t n) {
    std::lock_guard<std::mutex> lock(out);
    auto bytes = static_cast<uint32_t>(n * 4);
    bool ok = put(&id, 8) && put(&bytes, 4) && (n == 0 || put(v, n * 4));
    return ok && std::fflush(wire) == 0 ? 0 : 1;
}

// "1,2,3" into `ids`; false on anything but decimal ids and commas
bool parse_ids(const char *s, std::vector<uint64_t> &ids) {
    while (*s != '\0') {
        char *end = nullptr;
        uint64_t id = std::strtoull(s, &end, 10);
        if (end == s || (*end != ',' && *end != '\0')) {
            return false;
        }
        ids.push_back(id);
        s = *end == ',' ? end + 1 : end;
    }
    return true;
}

} // namespace

int main(int argc, char **argv) {
    if (argc < 2) {
        std::fputs("usage: tridi-step-helper PATH [--coarse] [--only ID,...]\n", stderr);
        return USAGE;
    }
    // die with the parent: a viewer killed while waiting must not leave us running
    prctl(PR_SET_PDEATHSIG, SIGKILL);
    int coarse = 0;
    std::vector<uint64_t> only;
    for (int i = 2; i < argc; i++) {
        if (std::strcmp(argv[i], "--coarse") == 0) {
            coarse = 1;
        } else if (std::strcmp(argv[i], "--only") == 0 && i + 1 < argc && parse_ids(argv[i + 1], only)) {
            i++;
        } else {
            std::fprintf(stderr, "tridi-step-helper: bad argument %s\n", argv[i]);
            return USAGE;
        }
    }
    int fd = dup(1);
    if (fd < 0 || dup2(2, 1) < 0 || (wire = fdopen(fd, "wb")) == nullptr) {
        std::perror("tridi-step-helper: stdout");
        return WRITE;
    }
    return tridi_step_tessellate(argv[1], coarse, only.data(), only.size(), nullptr, on_ids, on_part);
}
