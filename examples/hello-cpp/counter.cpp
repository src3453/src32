#include "counter.h"

namespace demo {
Counter::Counter(uint32_t initial) : value_(initial) {}

uint32_t Counter::advance(uint32_t step) {
    value_ += step;
    return value_;
}
}
