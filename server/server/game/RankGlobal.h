//RankGlobal
#include "../common/prodomodefines.h"
#ifdef ENABLE_GLOBAL_RANK

void RankGlobal_send_players(LPCHARACTER ch);
void RankGlobal_send_items(LPCHARACTER ch);
void RankGlobal_end_season(LPCHARACTER ch);
void RankGlobal_add_point(LPCHARACTER ch, BYTE type, int count);

void RankGlobal_DB_load_player(stRankGlobal_player * data);
void RankGlobal_DB_load_item(stRankGlobal_item * data);
void RankGlobal_DB_load_state(stRankGlobal_state * data);
#endif