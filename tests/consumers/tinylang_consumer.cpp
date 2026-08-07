#include "tinylang.h"

#include <cstring>

int main() {
    if (tinyone_abi_version() != TINYONE_ABI_VERSION) {
        return 10;
    }

    char *response = tinyone_compile_source_json("print 7");
    if (response == nullptr || std::strstr(response, "\"ok\":true") == nullptr) {
        tinyone_free_string(response);
        return 11;
    }
    tinyone_free_string(response);
    return 0;
}
