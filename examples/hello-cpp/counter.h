#ifndef HELLO_CPP_COUNTER_H
#define HELLO_CPP_COUNTER_H

#include <stdint.h>

namespace demo {
constexpr uint32_t expected = 42;

// Out-of-line methods avoid the backend's unsupported COMDAT linkage.
class Counter {
public:
    explicit Counter(uint32_t initial);
    uint32_t advance(uint32_t step);

private:
    uint32_t value_;
};
}

#endif
