#ifndef Bridge_h
#define Bridge_h
#include <stdint.h>
#include <stddef.h>
int32_t wifi_share_start(uint16_t port);
void wifi_share_stop(void);
int32_t wifi_share_running(void);
uint32_t wifi_share_active(void);
int32_t wifi_share_linked(void);
int32_t wifi_share_pc_name(char *buf, size_t cap);
int32_t wifi_share_last_error(char *buf, size_t cap);
void wifi_share_set_device_name(const char *name);
#endif
