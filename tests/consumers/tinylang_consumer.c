#include "tinylang.h"

#include <stdio.h>

int main(void) {
    if (tinyone_abi_version() != TINYONE_ABI_VERSION) {
        return 10;
    }

    char *response = tinyone_run_source_json("print 42", "vm", NULL);
    if (response == NULL) {
        return 11;
    }
    puts(response);
    tinyone_free_string(response);
    tinyone_free_string(NULL);
    return 0;
}
