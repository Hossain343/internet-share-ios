#ifndef Bridge_h
#define Bridge_h
#include <stdint.h>
int32_t wifi_share_start(uint16_t port);
void wifi_share_stop(void);
int32_t wifi_share_running(void);
uint32_t wifi_share_active(void);
#endif
