// A separate Clang translation unit with relative32 Itanium vtables.
#include <typeinfo>

namespace relative_fixture {
struct Primary {
    virtual int first(int amount) = 0;
    virtual int next(int amount) = 0;
};
struct Side {
    virtual int second(int amount) = 0;
};
struct RelativeObject final : Primary, Side {
    int value = 7;
    int first(int amount) override { return value + amount; }
    int next(int amount) override { return value + amount * 2; }
    int second(int amount) override { return value + amount * 3; }
};
RelativeObject& sample() {
    static RelativeObject object;
    return object;
}
}

extern "C" void* cppvtable_relative_primary() {
    return static_cast<relative_fixture::Primary*>(&relative_fixture::sample());
}
extern "C" void* cppvtable_relative_side() {
    return static_cast<relative_fixture::Side*>(&relative_fixture::sample());
}
extern "C" const void* cppvtable_relative_type() {
    return &typeid(relative_fixture::RelativeObject);
}
extern "C" bool cppvtable_relative_typeid(void* raw) {
    return typeid(*static_cast<relative_fixture::Side*>(raw)) ==
           typeid(relative_fixture::RelativeObject);
}
