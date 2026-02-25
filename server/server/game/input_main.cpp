#include "stdafx.h"
#include "constants.h"
#include "config.h"
#include "utils.h"
#include "desc_client.h"
#include "desc_manager.h"
#include "buffer_manager.h"
#include "packet.h"
#include "protocol.h"
#include "char.h"
#include "char_manager.h"
#include "item.h"
#include "item_manager.h"
#include "cmd.h"
#include "shop.h"
#include "shop_manager.h"
#include "safebox.h"
#include "regen.h"
#include "battle.h"
#include "exchange.h"
#include "questmanager.h"
#include "profiler.h"
#include "messenger_manager.h"
#include "party.h"
#include "p2p.h"
#include "affect.h"
#include "guild.h"
#include "guild_manager.h"
#include "log.h"
#include "banword.h"
#include "empire_text_convert.h"
#include "unique_item.h"
#include "building.h"
#include "locale_service.h"
#include "gm.h"
#include "spam.h"
#include "ani.h"
#include "motion.h"
#include "OXEvent.h"
#include "locale_service.h"
#include "DragonSoul.h"
#include "belt_inventory_helper.h"
#include "Biologist.h"
#include "refactorized_switchbot.h"
#include "private_shop_manager.h"
#include "private_shop.h"
#include "private_shop_util.h"
#include "input.h"
#include "event_manager.h"
#include "BattlePassManager.h"
#include "../common/prodomodefines.h"
#define ENABLE_CHAT_COLOR_SYSTEM
#define ENABLE_CHAT_SPAMLIMIT
#define ENABLE_WHISPER_CHAT_SPAMLIMIT
#define ENABLE_CHECK_GHOSTMODE

#ifdef ENABLE_REMOVE_LIMIT_GOLD
static unsigned long long __deposit_limit()
#else
static int __deposit_limit()
#endif
{
	return (1000*10000); // 1õ��
}

#ifdef __SEND_TARGET_INFO__
void CInputMain::TargetInfoLoad(LPCHARACTER ch, const char* c_pData)
{
	TPacketCGTargetInfoLoad* p = (TPacketCGTargetInfoLoad*)c_pData;
	TPacketGCTargetInfo pInfo;
	pInfo.header = HEADER_GC_TARGET_INFO;
	static std::vector<LPITEM> s_vec_item;
	s_vec_item.clear();
	LPITEM pkInfoItem;
	LPCHARACTER m_pkChrTarget = CHARACTER_MANAGER::instance().Find(p->dwVID);

	if (!ch || !m_pkChrTarget)
	return;

	if (ITEM_MANAGER::instance().CreateDropItemVector(m_pkChrTarget, ch, s_vec_item) && (m_pkChrTarget->IsMonster() || m_pkChrTarget->IsStone()))
	{
		if (s_vec_item.size() == 0);
		else if (s_vec_item.size() == 1)
		{
			pkInfoItem = s_vec_item[0];
			pInfo.dwVID	= m_pkChrTarget->GetVID();
			pInfo.race = m_pkChrTarget->GetRaceNum();
			pInfo.dwVnum = pkInfoItem->GetVnum();
			pInfo.count = pkInfoItem->GetCount();
#ifdef ENABLE_SEND_TARGET_INFO_EXTENDED
			pInfo.rarity = pkInfoItem->GetRarity();
#endif
			ch->GetDesc()->Packet(&pInfo, sizeof(TPacketGCTargetInfo));
		}
		else
		{
			int option = 2;
			std::sort(s_vec_item.begin(), s_vec_item.end(), [option](const LPITEM i1, const LPITEM i2) {
				switch (option) {
					case 2:
						return i1->CustomSort() == i2->CustomSort() ? i1->GetSubType() < i2->GetSubType() : i1->CustomSort() < i2->CustomSort();
					default:
						return std::strcmp(i1->GetName(), i2->GetName()) < 0;
				}
			});

			for (const auto& getitem : s_vec_item) {
				pInfo.dwVID	= m_pkChrTarget->GetVID();
				pInfo.race = m_pkChrTarget->GetRaceNum();
				pInfo.dwVnum = getitem->GetVnum();
				
				if (getitem->GetType() == ITEM_METIN)
					continue;
				
				pInfo.count = getitem->GetCount();
#ifdef ENABLE_SEND_TARGET_INFO_EXTENDED
				pInfo.rarity = getitem->GetRarity();
#endif
				ch->GetDesc()->Packet(&pInfo, sizeof(TPacketGCTargetInfo));
			};
		}
	}
}
#endif

void SendBlockChatInfo(LPCHARACTER ch, int sec)
{
	if (sec <= 0)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;652]");
		return;
	}

	long hour = sec / 3600;
	sec -= hour * 3600;

	long min = (sec / 60);
	sec -= min * 60;

	char buf[128+1];

#ifdef __MULTI_LANGUAGE_SYSTEM__
	if (hour > 0 && min > 0)
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[LS;1028;%d;%d;%d]"), hour, min, sec);
	else if (hour > 0 && min == 0)
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[LS;1029;%d;%d]"), hour, sec);
	else if (hour == 0 && min > 0)
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[LS;1030;%d;%d]"), min, sec);
	else
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("[LS;1042;%d]"), sec);
#endif

	ch->ChatPacket(CHAT_TYPE_INFO, buf);
}

EVENTINFO(spam_event_info)
{
	char host[MAX_HOST_LENGTH+1];

	spam_event_info()
	{
		::memset( host, 0, MAX_HOST_LENGTH+1 );
	}
};

typedef boost::unordered_map<std::string, std::pair<unsigned int, LPEVENT> > spam_score_of_ip_t;
spam_score_of_ip_t spam_score_of_ip;

EVENTFUNC(block_chat_by_ip_event)
{
	spam_event_info* info = dynamic_cast<spam_event_info*>( event->info );

	if ( info == NULL )
	{
		sys_err( "block_chat_by_ip_event> <Factor> Null pointer" );
		return 0;
	}

	const char * host = info->host;

	spam_score_of_ip_t::iterator it = spam_score_of_ip.find(host);

	if (it != spam_score_of_ip.end())
	{
		it->second.first = 0;
		it->second.second = NULL;
	}

	return 0;
}

bool SpamBlockCheck(LPCHARACTER ch, const char* const buf, const size_t buflen)
{
	if (ch->GetLevel() < g_iSpamBlockMaxLevel)
	{
		spam_score_of_ip_t::iterator it = spam_score_of_ip.find(ch->GetDesc()->GetHostName());

		if (it == spam_score_of_ip.end())
		{
			spam_score_of_ip.insert(std::make_pair(ch->GetDesc()->GetHostName(), std::make_pair(0, (LPEVENT) NULL)));
			it = spam_score_of_ip.find(ch->GetDesc()->GetHostName());
		}

		if (it->second.second)
		{
			SendBlockChatInfo(ch, event_time(it->second.second) / passes_per_sec);
			return true;
		}

		unsigned int score;
		const char * word = SpamManager::instance().GetSpamScore(buf, buflen, score);

		it->second.first += score;

		if (word)
			sys_log(0, "SPAM_SCORE: %s text: %s score: %u total: %u word: %s", ch->GetName(), buf, score, it->second.first, word);

		if (it->second.first >= g_uiSpamBlockScore)
		{
			spam_event_info* info = AllocEventInfo<spam_event_info>();
			strlcpy(info->host, ch->GetDesc()->GetHostName(), sizeof(info->host));

			it->second.second = event_create(block_chat_by_ip_event, info, PASSES_PER_SEC(g_uiSpamBlockDuration));
			sys_log(0, "SPAM_IP: %s for %u seconds", info->host, g_uiSpamBlockDuration);

			LogManager::instance().CharLog(ch, 0, "SPAM", word);

			SendBlockChatInfo(ch, event_time(it->second.second) / passes_per_sec);

			return true;
		}
	}

	return false;
}

enum
{
	TEXT_TAG_PLAIN,
	TEXT_TAG_TAG, // ||
	TEXT_TAG_COLOR, // |cffffffff
	TEXT_TAG_HYPERLINK_START, // |H
	TEXT_TAG_HYPERLINK_END, // |h ex) |Hitem:1234:1:1:1|h
	TEXT_TAG_RESTORE_COLOR,
};

int GetTextTag(const char * src, int maxLen, int & tagLen, std::string & extraInfo)
{
	tagLen = 1;

	if (maxLen < 2 || *src != '|')
		return TEXT_TAG_PLAIN;

	const char * cur = ++src;

	if (*cur == '|') // ||�� |�� ǥ���Ѵ�.
	{
		tagLen = 2;
		return TEXT_TAG_TAG;
	}
	else if (*cur == 'c') // color |cffffffffblahblah|r
	{
		tagLen = 2;
		return TEXT_TAG_COLOR;
	}
	else if (*cur == 'H') // hyperlink |Hitem:10000:0:0:0:0|h[�̸�]|h
	{
		tagLen = 2;
		return TEXT_TAG_HYPERLINK_START;
	}
	else if (*cur == 'h') // end of hyperlink
	{
		tagLen = 2;
		return TEXT_TAG_HYPERLINK_END;
	}

	return TEXT_TAG_PLAIN;
}

void GetTextTagInfo(const char * src, int src_len, int & hyperlinks, bool & colored)
{
	colored = false;
	hyperlinks = 0;

	int len;
	std::string extraInfo;

	for (int i = 0; i < src_len;)
	{
		int tag = GetTextTag(&src[i], src_len - i, len, extraInfo);

		if (tag == TEXT_TAG_HYPERLINK_START)
			++hyperlinks;

		if (tag == TEXT_TAG_COLOR)
			colored = true;

		i += len;
	}
}

int ProcessTextTag(LPCHARACTER ch, const char * c_pszText, size_t len)
{
   return 0;

   int hyperlinks;
   bool colored;

	GetTextTagInfo(c_pszText, len, hyperlinks, colored);

	if (colored == true && hyperlinks == 0)
		return 4;
	
	int nPrismCount = ch->CountSpecifyItem(ITEM_PRISM);

	if (nPrismCount < hyperlinks)
		return 1;


	if (!ch->GetMyShop())
	{
		ch->RemoveSpecifyItem(ITEM_PRISM, hyperlinks);
		return 0;
	} else
	{
		int sellingNumber = ch->GetMyShop()->GetNumberByVnum(ITEM_PRISM);
		if(nPrismCount - sellingNumber < hyperlinks)
		{
			return 2;
		} else
		{
			ch->RemoveSpecifyItem(ITEM_PRISM, hyperlinks);
			return 0;
		}
	}

	return 4;
}

int CInputMain::Whisper(LPCHARACTER ch, const char * data, size_t uiBytes)
{
	const TPacketCGWhisper* pinfo = reinterpret_cast<const TPacketCGWhisper*>(data);

	if (uiBytes < pinfo->wSize)
		return -1;

	int iExtraLen = pinfo->wSize - sizeof(TPacketCGWhisper);

	if (iExtraLen < 0)
	{
		sys_err("invalid packet length (len %d size %u buffer %u)", iExtraLen, pinfo->wSize, uiBytes);
		ch->GetDesc()->SetPhase(PHASE_CLOSE);
		return -1;
	}

#ifdef ENABLE_WHISPER_CHAT_SPAMLIMIT
	if (ch->IncreaseChatCounter() >= 10)
	{
		ch->GetDesc()->DelayedDisconnect(0);
		return (iExtraLen);
	}
#endif

	if (ch->FindAffect(AFFECT_BLOCK_CHAT))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;652]");
		return (iExtraLen);
	}

	LPCHARACTER pkChr = CHARACTER_MANAGER::instance().FindPC(pinfo->szNameTo);

	if (pkChr == ch)
		return (iExtraLen);

	LPDESC pkDesc = NULL;

	BYTE bOpponentEmpire = 0;

	if (test_server)
	{
		if (!pkChr)
			sys_log(0, "Whisper to %s(%s) from %s", "Null", pinfo->szNameTo, ch->GetName());
		else
			sys_log(0, "Whisper to %s(%s) from %s", pkChr->GetName(), pinfo->szNameTo, ch->GetName());
	}

	if (ch->IsBlockMode(BLOCK_WHISPER))
	{
		if (ch->GetDesc())
		{
			TPacketGCWhisper pack;
			pack.bHeader = HEADER_GC_WHISPER;
			pack.bType = WHISPER_TYPE_SENDER_BLOCKED;
			pack.wSize = sizeof(TPacketGCWhisper);
			strlcpy(pack.szNameFrom, pinfo->szNameTo, sizeof(pack.szNameFrom));
			ch->GetDesc()->Packet(&pack, sizeof(pack));
		}
		return iExtraLen;
	}

	if (!pkChr)
	{
		CCI * pkCCI = P2P_MANAGER::instance().Find(pinfo->szNameTo);

		if (pkCCI)
		{
			pkDesc = pkCCI->pkDesc;
			pkDesc->SetRelay(pinfo->szNameTo);
			bOpponentEmpire = pkCCI->bEmpire;

			if (test_server)
				sys_log(0, "Whisper to %s from %s (Channel %d Mapindex %d)", "Null", ch->GetName(), pkCCI->bChannel, pkCCI->lMapIndex);
		}
	}
	else
	{
		pkDesc = pkChr->GetDesc();
		bOpponentEmpire = pkChr->GetEmpire();
	}

	if (!pkDesc)
	{
		if (ch->GetDesc())
		{
#if defined(OFFLINE_MESSAGE_REWORKED)
			const BYTE bDelay = 10;
			char msg[64];
			if (get_dword_time() - ch->GetLastOfflinePMTime() > bDelay * 1000)
			{
				char buf[CHAT_MAX_LEN + 1];
				strlcpy(buf, data + sizeof(TPacketCGWhisper), MIN(iExtraLen + 1, sizeof(buf)));
				const size_t buflen = strlen(buf);

				// CBanwordManager::instance().ConvertString(buf, buflen);

				int processReturn = ProcessTextTag(ch, buf, buflen);
				if (0 != processReturn)
				{
					TItemTable* pTable = ITEM_MANAGER::instance().GetTable(ITEM_PRISM);

					if (pTable) {
						char buffer[128];
						int len;
						if (3 == processReturn)
							len = snprintf(buffer, sizeof(buffer), "[LS;1801]");
						else
							len = snprintf(buffer, sizeof(buffer), LC_TEXT("[#Unk]%s is required."), pTable->szLocaleName);

						if (len < 0 || len >= (int)sizeof(buffer))
							len = sizeof(buffer) - 1;

						++len;

						TPacketGCWhisper pack;

						pack.bHeader = HEADER_GC_WHISPER;
						pack.bType = WHISPER_TYPE_ERROR;
						pack.wSize = static_cast<WORD>(sizeof(TPacketGCWhisper) + len);
						strlcpy(pack.szNameFrom, pinfo->szNameTo, sizeof(pack.szNameFrom));

						ch->GetDesc()->BufferedPacket(&pack, sizeof(pack));
						ch->GetDesc()->Packet(buffer, len);

						sys_log(0, "WHISPER: not enough %s: char: %s", pTable->szLocaleName, ch->GetName());
					}
					return (iExtraLen);
				}

				if (buflen > 0) 
				{
					ch->SendOfflineMessage(pinfo->szNameTo, buf);
					snprintf(msg, sizeof(msg), "An offline message has been sent.");
				}
				else
					return (iExtraLen);
			}
			else
			{
				snprintf(msg, sizeof(msg), "You have to wait %d seconds for send offline message.", bDelay);
			}

			/*Info*/
			TPacketGCWhisper pack;
			int len = MIN(CHAT_MAX_LEN, strlen(msg) + 1);
			pack.bHeader = HEADER_GC_WHISPER;
			pack.wSize = static_cast<WORD>(sizeof(TPacketGCWhisper) + len);
			pack.bType = WHISPER_TYPE_SYSTEM;
			strlcpy(pack.szNameFrom, pinfo->szNameTo, sizeof(pack.szNameFrom));

			TEMP_BUFFER buf;
			buf.write(&pack, sizeof(TPacketGCWhisper));
			buf.write(msg, len);
			ch->GetDesc()->Packet(buf.read_peek(), buf.size());

#else
			TPacketGCWhisper pack;
			pack.bHeader = HEADER_GC_WHISPER;
			pack.bType = WHISPER_TYPE_NOT_EXIST;
			pack.wSize = sizeof(TPacketGCWhisper);
			strlcpy(pack.szNameFrom, pinfo->szNameTo, sizeof(pack.szNameFrom));
			ch->GetDesc()->Packet(&pack, sizeof(TPacketGCWhisper));
			sys_log(0, "WHISPER: no player");
#endif
		}
	}
	else
	{
		if (ch->IsBlockMode(BLOCK_WHISPER))
		{
			if (ch->GetDesc())
			{
				TPacketGCWhisper pack;
				pack.bHeader = HEADER_GC_WHISPER;
				pack.bType = WHISPER_TYPE_SENDER_BLOCKED;
				pack.wSize = sizeof(TPacketGCWhisper);
				strlcpy(pack.szNameFrom, pinfo->szNameTo, sizeof(pack.szNameFrom));
				ch->GetDesc()->Packet(&pack, sizeof(pack));
			}
		}
		else if (pkChr && pkChr->IsBlockMode(BLOCK_WHISPER))
		{
			if (ch->GetDesc())
			{
				TPacketGCWhisper pack;
				pack.bHeader = HEADER_GC_WHISPER;
				pack.bType = WHISPER_TYPE_TARGET_BLOCKED;
				pack.wSize = sizeof(TPacketGCWhisper);
				strlcpy(pack.szNameFrom, pinfo->szNameTo, sizeof(pack.szNameFrom));
				ch->GetDesc()->Packet(&pack, sizeof(pack));
			}
		}
		else
		{
			BYTE bType = WHISPER_TYPE_NORMAL;

			char buf[CHAT_MAX_LEN + 1];
			strlcpy(buf, data + sizeof(TPacketCGWhisper), MIN(iExtraLen + 1, sizeof(buf)));
			const size_t buflen = strlen(buf);

			if (true == SpamBlockCheck(ch, buf, buflen))
			{
				if (!pkChr)
				{
					CCI * pkCCI = P2P_MANAGER::instance().Find(pinfo->szNameTo);

					if (pkCCI)
					{
						pkDesc->SetRelay("");
					}
				}
				return iExtraLen;
			}

			// CBanwordManager::instance().ConvertString(buf, buflen);

			if (g_bEmpireWhisper)
				if (!ch->IsEquipUniqueGroup(UNIQUE_GROUP_RING_OF_LANGUAGE))
					if (!(pkChr && pkChr->IsEquipUniqueGroup(UNIQUE_GROUP_RING_OF_LANGUAGE)))
						if (bOpponentEmpire != ch->GetEmpire() && ch->GetEmpire() && bOpponentEmpire // ���� ������ �ٸ��鼭
								&& ch->GetGMLevel() == GM_PLAYER && gm_get_level(pinfo->szNameTo) == GM_PLAYER) // �Ѵ� �Ϲ� �÷��̾��̸�
							// �̸� �ۿ� �𸣴� gm_get_level �Լ��� ���
						{
							if (!pkChr)
							{
								// �ٸ� ������ ������ ���� ǥ�ø� �Ѵ�. bType�� ���� 4��Ʈ�� Empire��ȣ�� ����Ѵ�.
								bType = ch->GetEmpire() << 4;
							}
							else
							{
								ConvertEmpireText(ch->GetEmpire(), buf, buflen, 10 + 2 * pkChr->GetSkillPower(SKILL_LANGUAGE1 + ch->GetEmpire() - 1)/*��ȯȮ��*/);
							}
						}

			int processReturn = ProcessTextTag(ch, buf, buflen);
			if (0!=processReturn)
			{
				if (ch->GetDesc())
				{
					TItemTable * pTable = ITEM_MANAGER::instance().GetTable(ITEM_PRISM);

					if (pTable)
					{
						char buf[128];
						int len;
						if (3==processReturn) //교환중
							len = snprintf(buf, sizeof(buf), "[LS;432]");
						else
#ifdef __MULTI_LANGUAGE_SYSTEM__
							{
								BYTE bLocale = ch->GetDesc()->GetLanguage();
								len = snprintf(buf, sizeof(buf), LC_TEXT("%s [PDMULTILANG]UNK IS REQUIERED"), LC_LOCALE_ITEM_TEXT(ITEM_PRISM, bLocale));
							}
#endif

						if (len < 0 || len >= (int) sizeof(buf))
							len = sizeof(buf) - 1;

						++len;  // \0 ���� ����

						TPacketGCWhisper pack;

						pack.bHeader = HEADER_GC_WHISPER;
						pack.bType = WHISPER_TYPE_ERROR;
						pack.wSize = sizeof(TPacketGCWhisper) + len;
						strlcpy(pack.szNameFrom, pinfo->szNameTo, sizeof(pack.szNameFrom));

						ch->GetDesc()->BufferedPacket(&pack, sizeof(pack));
						ch->GetDesc()->Packet(buf, len);

						sys_log(0, "WHISPER: not enough %s: char: %s", pTable->szLocaleName, ch->GetName());
					}
				}

				// ������ ������ �� �����Ƿ� �����̸� Ǯ���ش�.
				pkDesc->SetRelay("");
				return (iExtraLen);
			}

			if (ch->IsGM())
				bType = (bType & 0xF0) | WHISPER_TYPE_GM;

			if (buflen > 0)
			{
				TPacketGCWhisper pack;

				pack.bHeader = HEADER_GC_WHISPER;
				pack.wSize = sizeof(TPacketGCWhisper) + buflen;
				pack.bType = bType;
				strlcpy(pack.szNameFrom, ch->GetName(), sizeof(pack.szNameFrom));
#if defined(__BL_CLIENT_LOCALE_STRING__)
				pack.bCanFormat = false;
#endif

				// desc->BufferedPacket�� ���� �ʰ� ���ۿ� ����ϴ� ������
				// P2P relay�Ǿ� ��Ŷ�� ĸ��ȭ �� �� �ֱ� �����̴�.
				TEMP_BUFFER tmpbuf;

				tmpbuf.write(&pack, sizeof(pack));
				tmpbuf.write(buf, buflen);

				pkDesc->Packet(tmpbuf.read_peek(), tmpbuf.size());
			}
		}
	}
	if(pkDesc)
		pkDesc->SetRelay("");

	return (iExtraLen);
}

struct RawPacketToCharacterFunc
{
	const void * m_buf;
	int	m_buf_len;

	RawPacketToCharacterFunc(const void * buf, int buf_len) : m_buf(buf), m_buf_len(buf_len)
	{
	}

	void operator () (LPCHARACTER c)
	{
		if (!c->GetDesc())
			return;

		c->GetDesc()->Packet(m_buf, m_buf_len);
	}
};

struct FEmpireChatPacket
{
	packet_chat& p;
	const char* orig_msg;
	int orig_len;
	char converted_msg[CHAT_MAX_LEN+1];

	BYTE bEmpire;
	int iMapIndex;
	int namelen;

	FEmpireChatPacket(packet_chat& p, const char* chat_msg, int len, BYTE bEmpire, int iMapIndex, int iNameLen)
		: p(p), orig_msg(chat_msg), orig_len(len), bEmpire(bEmpire), iMapIndex(iMapIndex), namelen(iNameLen)
	{
		memset( converted_msg, 0, sizeof(converted_msg) );
	}

	void operator () (LPDESC d)
	{
		if (!d->GetCharacter())
			return;

		if (d->GetCharacter()->GetMapIndex() != iMapIndex)
			return;

		d->BufferedPacket(&p, sizeof(packet_chat));

		if (d->GetEmpire() == bEmpire ||
			bEmpire == 0 ||
			d->GetCharacter()->GetGMLevel() > GM_PLAYER ||
			d->GetCharacter()->IsEquipUniqueGroup(UNIQUE_GROUP_RING_OF_LANGUAGE))
		{
			d->Packet(orig_msg, orig_len);
		}
		else
		{
			// ������� ��ų������ �ٸ��� �Ź� �ؾ��մϴ�
			size_t len = strlcpy(converted_msg, orig_msg, sizeof(converted_msg));

			if (len >= sizeof(converted_msg))
				len = sizeof(converted_msg) - 1;

			ConvertEmpireText(bEmpire, converted_msg + namelen, len - namelen, 10 + 2 * d->GetCharacter()->GetSkillPower(SKILL_LANGUAGE1 + bEmpire - 1));
			d->Packet(converted_msg, orig_len);
		}
	}
};

struct FYmirChatPacket
{
	packet_chat& packet;
	const char* m_szChat;
	size_t m_lenChat;
	const char* m_szName;

	int m_iMapIndex;
	BYTE m_bEmpire;
	bool m_ring;

	char m_orig_msg[CHAT_MAX_LEN+1];
	int m_len_orig_msg;
	char m_conv_msg[CHAT_MAX_LEN+1];
	int m_len_conv_msg;

	FYmirChatPacket(packet_chat& p, const char* chat, size_t len_chat, const char* name, size_t len_name, int iMapIndex, BYTE empire, bool ring)
		: packet(p),
		m_szChat(chat), m_lenChat(len_chat),
		m_szName(name),
		m_iMapIndex(iMapIndex), m_bEmpire(empire),
		m_ring(ring)
	{
		m_len_orig_msg = snprintf(m_orig_msg, sizeof(m_orig_msg), "%s : %s", m_szName, m_szChat) + 1; // �� ���� ����

		if (m_len_orig_msg < 0 || m_len_orig_msg >= (int) sizeof(m_orig_msg))
			m_len_orig_msg = sizeof(m_orig_msg) - 1;

		m_len_conv_msg = snprintf(m_conv_msg, sizeof(m_conv_msg), "??? : %s", m_szChat) + 1; // �� ���� ������

		if (m_len_conv_msg < 0 || m_len_conv_msg >= (int) sizeof(m_conv_msg))
			m_len_conv_msg = sizeof(m_conv_msg) - 1;

		ConvertEmpireText(m_bEmpire, m_conv_msg + 6, m_len_conv_msg - 6, 10); // 6�� "??? : "�� ����
	}

	void operator() (LPDESC d)
	{
		if (!d->GetCharacter())
			return;

		if (d->GetCharacter()->GetMapIndex() != m_iMapIndex)
			return;

		if (m_ring ||
			d->GetEmpire() == m_bEmpire ||
			d->GetCharacter()->GetGMLevel() > GM_PLAYER ||
			d->GetCharacter()->IsEquipUniqueGroup(UNIQUE_GROUP_RING_OF_LANGUAGE))
		{
			packet.size = m_len_orig_msg + sizeof(TPacketGCChat);

			d->BufferedPacket(&packet, sizeof(packet_chat));
			d->Packet(m_orig_msg, m_len_orig_msg);
		}
		else
		{
			packet.size = m_len_conv_msg + sizeof(TPacketGCChat);

			d->BufferedPacket(&packet, sizeof(packet_chat));
			d->Packet(m_conv_msg, m_len_conv_msg);
		}
	}
};

int CInputMain::Chat(LPCHARACTER ch, const char * data, size_t uiBytes)
{
	const TPacketCGChat* pinfo = reinterpret_cast<const TPacketCGChat*>(data);

	if (uiBytes < pinfo->size)
		return -1;

	const int iExtraLen = pinfo->size - sizeof(TPacketCGChat);

	if (iExtraLen < 0)
	{
		sys_err("invalid packet length (len %d size %u buffer %u)", iExtraLen, pinfo->size, uiBytes);
		ch->GetDesc()->SetPhase(PHASE_CLOSE);
		return -1;
	}


	char buf[CHAT_MAX_LEN - (CHARACTER_NAME_MAX_LEN + 3) + 1];
	strlcpy(buf, data + sizeof(TPacketCGChat), MIN(iExtraLen + 1, sizeof(buf)));
	const size_t buflen = strlen(buf);

	if (buflen > 1 && *buf == '/')
	{
		interpret_command(ch, buf + 1, buflen - 1);
		return iExtraLen;
	}
#ifdef ENABLE_CHAT_SPAMLIMIT
	if (ch->IncreaseChatCounter() >= 4)
	{
		if (ch->GetChatCounter() == 10)
			ch->GetDesc()->DelayedDisconnect(0);
		return iExtraLen;
	}
#else
	if (ch->IncreaseChatCounter() >= 10)
	{
		if (ch->GetChatCounter() == 10)
		{
			sys_log(0, "CHAT_HACK: %s", ch->GetName());
			ch->GetDesc()->DelayedDisconnect(5);
		}

		return iExtraLen;
	}
#endif

	// ä�� ���� Affect ó��
	const CAffect* pAffect = ch->FindAffect(AFFECT_BLOCK_CHAT);

	if (pAffect != NULL)
	{
		SendBlockChatInfo(ch, pAffect->lDuration);
		return iExtraLen;
	}

	if (true == SpamBlockCheck(ch, buf, buflen))
	{
		return iExtraLen;
	}

	// @fixme133 begin
	// CBanwordManager::instance().ConvertString(buf, buflen);

	int processReturn = ProcessTextTag(ch, buf, buflen);
	if (0!=processReturn)
	{
		const TItemTable* pTable = ITEM_MANAGER::instance().GetTable(ITEM_PRISM);

		if (NULL != pTable)
		{
			if (3==processReturn) //교환중
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;432]");
			else
#ifdef __MULTI_LANGUAGE_SYSTEM__
				{
					BYTE bLocale = ch->GetDesc()->GetLanguage();
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("%s bla bla bla"), LC_LOCALE_ITEM_TEXT(ITEM_PRISM, bLocale));
				}
#endif

		}

		return iExtraLen;
	}
	// @fixme133 end

	char chatbuf[CHAT_MAX_LEN + 1];
	int len = snprintf(chatbuf, sizeof(chatbuf), "%s : %s", ch->GetName(), buf);


	if (CHAT_TYPE_SHOUT == pinfo->type)
	{
#ifdef ENABLE_BATTLE_PASS
		CBattlePass::Instance().RegisterDefaultMission(MISSION_TYPE_MESSAGES, 1, ch);
#endif
		LogManager::instance().ShoutLog(g_bChannel, ch->GetEmpire(), chatbuf);
	}

	if (len < 0 || len >= (int) sizeof(chatbuf))
		len = sizeof(chatbuf) - 1;

	if (pinfo->type == CHAT_TYPE_SHOUT)
	{
		// const int SHOUT_LIMIT_LEVEL = 15;

		if (ch->GetLevel() < g_iShoutLimitLevel)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Shout can only be used at level %d or higher."), g_iShoutLimitLevel);
			return (iExtraLen);
		}

		// if (thecore_heart->pulse - (int) ch->GetLastShoutPulse() < passes_per_sec * g_iShoutLimitTime)
		if (thecore_heart->pulse - (int) ch->GetLastShoutPulse() < passes_per_sec * 15)
			return (iExtraLen);


		int len = snprintf(chatbuf, sizeof(chatbuf), "|L%s|l %s : %s", LC_LOCALE(ch->GetDesc()->GetLanguage()), ch->GetName(), buf);




		ch->SetLastShoutPulse(thecore_heart->pulse);
		TPacketGGShout p;

		p.bHeader = HEADER_GG_SHOUT;
		p.bEmpire = ch->GetEmpire();
		strlcpy(p.szText, chatbuf, sizeof(p.szText));

		P2P_MANAGER::instance().Send(&p, sizeof(TPacketGGShout));

		SendShout(chatbuf, ch->GetEmpire());

		return (iExtraLen);
	}

	TPacketGCChat pack_chat;

	pack_chat.header = HEADER_GC_CHAT;
	pack_chat.size = sizeof(TPacketGCChat) + len;
	pack_chat.type = pinfo->type;
	pack_chat.id = ch->GetVID();
#if defined(LOCALE_STRING_RENEWAL)
	pack_chat.bCanFormat = false;
#endif

	switch (pinfo->type)
	{
		case CHAT_TYPE_TALKING:
			{
				const DESC_MANAGER::DESC_SET & c_ref_set = DESC_MANAGER::instance().GetClientSet();

				if (false)
				{
					std::for_each(c_ref_set.begin(), c_ref_set.end(),
							FYmirChatPacket(pack_chat,
								buf,
								strlen(buf),
								ch->GetName(),
								strlen(ch->GetName()),
								ch->GetMapIndex(),
								ch->GetEmpire(),
								ch->IsEquipUniqueGroup(UNIQUE_GROUP_RING_OF_LANGUAGE)));
				}
				else
				{
					std::for_each(c_ref_set.begin(), c_ref_set.end(),
							FEmpireChatPacket(pack_chat,
								chatbuf,
								len,
								(ch->GetGMLevel() > GM_PLAYER ||
								 ch->IsEquipUniqueGroup(UNIQUE_GROUP_RING_OF_LANGUAGE)) ? 0 : ch->GetEmpire(),
								ch->GetMapIndex(), strlen(ch->GetName())));
				}
			}
			break;

		case CHAT_TYPE_PARTY:
			{
				if (!ch->GetParty())
					ch->ChatPacket(CHAT_TYPE_INFO, "[LS;655]");
				else
				{
					TEMP_BUFFER tbuf;

					tbuf.write(&pack_chat, sizeof(pack_chat));
					tbuf.write(chatbuf, len);

					RawPacketToCharacterFunc f(tbuf.read_peek(), tbuf.size());
					ch->GetParty()->ForEachOnlineMember(f);
				}
			}
			break;

		case CHAT_TYPE_GUILD:
			{
				if (!ch->GetGuild())
					ch->ChatPacket(CHAT_TYPE_INFO, "[LS;656]");
				else
				{
					ch->GetGuild()->Chat(chatbuf);
				}
			}
			break;

		default:
			sys_err("Unknown chat type %d", pinfo->type);
			break;
	}

	return (iExtraLen);
}

void CInputMain::ItemUse(LPCHARACTER ch, const char * data)
{
#ifdef ENABLE_REWARD_SYSTEM
	LPITEM item = ch->GetItem(((struct command_item_use*)data)->Cell);
	const DWORD itemVnum = item ? item->GetVnum() : 0;
	const bool isSuccess = ch->UseItem(((struct command_item_use*)data)->Cell);
	if (isSuccess)
		CHARACTER_MANAGER::Instance().DoReward(ch, REWARD_MISSION_USE_ITEM, itemVnum, 1);
#else
	ch->UseItem(((struct command_item_use*)data)->Cell);
#endif
}

void CInputMain::ItemToItem(LPCHARACTER ch, const char * pcData)
{
	TPacketCGItemUseToItem * p = (TPacketCGItemUseToItem *) pcData;
#ifdef ENABLE_REWARD_SYSTEM
	if (ch)
	{
		LPITEM item = ch->GetItem(p->Cell);
		const DWORD itemVnum = item ? item->GetVnum() : 0;
		const bool isSuccess = ch->UseItem(p->Cell, p->TargetCell);
		if (isSuccess)
			CHARACTER_MANAGER::Instance().DoReward(ch, REWARD_MISSION_USE_ITEM, itemVnum, 1);
	}
#else
	if (ch)
		ch->UseItem(p->Cell, p->TargetCell);
#endif
}

void CInputMain::ItemDrop(LPCHARACTER ch, const char * data)
{
	struct command_item_drop * pinfo = (struct command_item_drop *) data;

	//MONARCH_LIMIT
	//if (ch->IsMonarch())
	//	return;
	//END_MONARCH_LIMIT
	if (!ch)
		return;

	// ��ũ�� 0���� ũ�� ��ũ�� ������ �� �̴�.
	if (pinfo->gold > 0)
		ch->DropGold(pinfo->gold);
	else
		ch->DropItem(pinfo->Cell);
}

void CInputMain::ItemDrop2(LPCHARACTER ch, const char * data)
{
	TPacketCGItemDrop2 * pinfo = (TPacketCGItemDrop2 *) data;
	if (!ch)
		return;
	if (pinfo->gold > 0)
		ch->DropGold(pinfo->gold);
	else
		ch->DropItem(pinfo->Cell, pinfo->count);
}

void CInputMain::ItemDestroy(LPCHARACTER ch, const char * data)
{
	struct command_item_destroy * pinfo = (struct command_item_destroy *) data;
	if (ch)
		ch->DestroyItem(pinfo->Cell);
}

void CInputMain::ItemMove(LPCHARACTER ch, const char * data)
{
	struct command_item_move * pinfo = (struct command_item_move *) data;

	if (ch)
		ch->MoveItem(pinfo->Cell, pinfo->CellTo, pinfo->count);
}

#ifdef ENABLE_EXTEND_INVEN_SYSTEM
void CInputMain::InventoryExpansion(LPCHARACTER ch, const char * data)
{
    if (ch) 
		ch->Update_Inven();
}
#endif

void CInputMain::ItemPickup(LPCHARACTER ch, const char * data)
{
	struct command_item_pickup * pinfo = (struct command_item_pickup*) data;
	if (ch)
		ch->PickupItem(pinfo->vid);
}

void CInputMain::QuickslotAdd(LPCHARACTER ch, const char* data)
{
	if (!ch)
		return;

	TPacketCGQuickslotAdd* pinfo = (TPacketCGQuickslotAdd*)data;

	if (pinfo->slot.type == QUICKSLOT_TYPE_ITEM)	//@fixme528
	{
		LPITEM item = nullptr;
		const TItemPos srcCell(INVENTORY, pinfo->slot.pos);

		if (!(item = ch->GetItem(srcCell)))
			return;
		
		// unused for now.
		/*if (IS_SET(item->GetAntiFlag(), ITEM_ANTIFLAG_QUICKSLOT))
			return; */

		switch (item->GetType())
		{
			case ITEM_USE:
				break;

			default:
				return;
		}
	}

	ch->SetQuickslot(pinfo->pos, pinfo->slot);
}


void CInputMain::QuickslotDelete(LPCHARACTER ch, const char * data)
{
	struct command_quickslot_del * pinfo = (struct command_quickslot_del *) data;
	ch->DelQuickslot(pinfo->pos);
}

void CInputMain::QuickslotSwap(LPCHARACTER ch, const char * data)
{
	struct command_quickslot_swap * pinfo = (struct command_quickslot_swap *) data;
	ch->SwapQuickslot(pinfo->pos, pinfo->change_pos);
}

int CInputMain::Messenger(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	TPacketCGMessenger* p = (TPacketCGMessenger*) c_pData;

	if (uiBytes < sizeof(TPacketCGMessenger))
		return -1;

	c_pData += sizeof(TPacketCGMessenger);
	uiBytes -= sizeof(TPacketCGMessenger);

	switch (p->subheader)
	{
		case MESSENGER_SUBHEADER_CG_ADD_BY_VID:
			{
				if (uiBytes < sizeof(TPacketCGMessengerAddByVID))
					return -1;

				TPacketCGMessengerAddByVID * p2 = (TPacketCGMessengerAddByVID *) c_pData;
				LPCHARACTER ch_companion = CHARACTER_MANAGER::instance().Find(p2->vid);

				if (!ch_companion)
					return sizeof(TPacketCGMessengerAddByVID);

				if (ch->IsObserverMode())
					return sizeof(TPacketCGMessengerAddByVID);

				if (ch_companion->IsBlockMode(BLOCK_MESSENGER_INVITE))
				{
					ch->ChatPacket(CHAT_TYPE_INFO, "[LS;657]");
					return sizeof(TPacketCGMessengerAddByVID);
				}

				LPDESC d = ch_companion->GetDesc();

				if (!d)
					return sizeof(TPacketCGMessengerAddByVID);

				if (ch->GetGMLevel() == GM_PLAYER && ch_companion->GetGMLevel() != GM_PLAYER)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, "[LS;659]");
					return sizeof(TPacketCGMessengerAddByVID);
				}

				if (ch->GetDesc() == d) // 자신은 추가할 수 없다.
					return sizeof(TPacketCGMessengerAddByVID);

				MessengerManager::instance().RequestToAdd(ch, ch_companion);
				//MessengerManager::instance().AddToList(ch->GetName(), ch_companion->GetName());
			}
			return sizeof(TPacketCGMessengerAddByVID);

		case MESSENGER_SUBHEADER_CG_ADD_BY_NAME:
			{
				if (uiBytes < CHARACTER_NAME_MAX_LEN)
					return -1;

				char name[CHARACTER_NAME_MAX_LEN + 1];
				strlcpy(name, c_pData, sizeof(name));

				if (ch->GetGMLevel() == GM_PLAYER && gm_get_level(name) != GM_PLAYER)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, "[LS;659]");
					return CHARACTER_NAME_MAX_LEN;
				}

				LPCHARACTER tch = CHARACTER_MANAGER::instance().FindPC(name);

				if (!tch)
					ch->ChatPacket(CHAT_TYPE_INFO, "[LS;660;%s]", name);
				else
				{
					if (tch == ch) // 자신은 추가할 수 없다.
						return CHARACTER_NAME_MAX_LEN;

					if (tch->IsBlockMode(BLOCK_MESSENGER_INVITE) == true)
					{
						ch->ChatPacket(CHAT_TYPE_INFO, "[LS;657]");
					}
					else
					{
						// �޽����� ĳ���ʹ����� �Ǹ鼭 ����
						MessengerManager::instance().RequestToAdd(ch, tch);
						//MessengerManager::instance().AddToList(ch->GetName(), tch->GetName());
					}
				}
			}
			return CHARACTER_NAME_MAX_LEN;

		case MESSENGER_SUBHEADER_CG_REMOVE:
			{
				if (uiBytes < CHARACTER_NAME_MAX_LEN)
					return -1;

				char char_name[CHARACTER_NAME_MAX_LEN + 1];
				strlcpy(char_name, c_pData, sizeof(char_name));
				MessengerManager::instance().RemoveFromList(char_name, ch->GetName());
			}
			return CHARACTER_NAME_MAX_LEN;

		default:
			sys_err("CInputMain::Messenger : Unknown subheader %d : %s", p->subheader, ch->GetName());
			break;
	}

	return 0;
}

typedef struct fckOFF
{
BYTE bySlot;
WORD byCount;
} TfckOFF;

int CInputMain::Shop(LPCHARACTER ch, const char * data, size_t uiBytes)
{
TPacketCGShop * p = (TPacketCGShop *) data;

if (uiBytes < sizeof(TPacketCGShop))
return -1;

if (test_server)
sys_log(0, "CInputMain::Shop() ==> SubHeader %d", p->subheader);

const char * c_pData = data + sizeof(TPacketCGShop);
uiBytes -= sizeof(TPacketCGShop);

switch (p->subheader)
{
case SHOP_SUBHEADER_CG_END:
sys_log(1, "INPUT: %s SHOP: END", ch->GetName());
CShopManager::instance().StopShopping(ch);
return 0;

case SHOP_SUBHEADER_CG_BUY:
{
if (uiBytes < sizeof(BYTE) + sizeof(BYTE))
return -1;

BYTE bPos = *(c_pData + 1);
sys_log(1, "INPUT: %s SHOP: BUY %d", ch->GetName(), bPos);
CShopManager::instance().Buy(ch, bPos);
return (sizeof(BYTE) + sizeof(BYTE));
}

case SHOP_SUBHEADER_CG_SELL:
{
if (uiBytes < sizeof(BYTE))
return -1;

BYTE pos = *c_pData;

sys_log(0, "INPUT: %s SHOP: SELL", ch->GetName());
CShopManager::instance().Sell(ch, pos);
return sizeof(BYTE);
}

case SHOP_SUBHEADER_CG_SELL2:
{
if (uiBytes < sizeof(TfckOFF))
return -1;
TfckOFF*p2 = (TfckOFF*)c_pData;
/*BYTE pos = *(c_pData++);
WORD count = *(c_pData);*/

sys_log(0, "INPUT: %s SHOP: SELL2", ch->GetName());
CShopManager::instance().Sell(ch, p2->bySlot, p2->byCount);
return sizeof(TfckOFF);
}

default:
sys_err("CInputMain::Shop : Unknown subheader %d : %s", p->subheader, ch->GetName());
break;
}

return 0;
}

void CInputMain::OnClick(LPCHARACTER ch, const char * data)
{
	struct command_on_click *	pinfo = (struct command_on_click *) data;
	LPCHARACTER			victim;

	if ((victim = CHARACTER_MANAGER::instance().Find(pinfo->vid)))
		victim->OnClick(ch);
	else if (test_server)
	{
		sys_err("CInputMain::OnClick %s.Click.NOT_EXIST_VID[%d]", ch->GetName(), pinfo->vid);
	}
}

#ifdef ENABLE_FISH_EVENT
int CInputMain::FishEvent(LPCHARACTER ch, const char * data, size_t uiBytes)
{
	TPacketCGFishEvent * p = (TPacketCGFishEvent *) data;

	if (uiBytes < sizeof(TPacketCGFishEvent))
		return -1;

	const char * c_pData = data + sizeof(TPacketCGFishEvent);
	uiBytes -= sizeof(TPacketCGFishEvent);

	switch (p->bSubheader)
	{
		case FISH_EVENT_SUBHEADER_BOX_USE:
			{
				if (uiBytes < sizeof(BYTE) + sizeof(WORD))
					return -1;

				BYTE bWindow = *(c_pData++);
				WORD wCell = *(c_pData);

				sys_log(0, "INPUT: %s FISH_EVENT: USE_BOX", ch->GetName());
				
				ch->FishEventUseBox(TItemPos(bWindow, wCell));
				return (sizeof(BYTE) + sizeof(WORD));
			}
			
		case FISH_EVENT_SUBHEADER_SHAPE_ADD:
			{
				if (uiBytes < sizeof(BYTE))
					return -1;

				BYTE shapePos = *c_pData;

				sys_log(0, "INPUT: %s FISH_EVENT: ADD_SHAPE", ch->GetName());
				
				ch->FishEventAddShape(shapePos);
				return sizeof(BYTE);
			}

		default:
			sys_err("CInputMain::FishEvent : Unknown subheader %d : %s", p->bSubheader, ch->GetName());
			break;
	}

	return 0;
}
#endif

void CInputMain::Exchange(LPCHARACTER ch, const char * data)
{
	struct command_exchange * pinfo = (struct command_exchange *) data;
	LPCHARACTER	to_ch = NULL;

	if (!ch->CanHandleItem())
		return;

	int iPulse = thecore_pulse();

	if ((to_ch = CHARACTER_MANAGER::instance().Find(pinfo->arg1)))
	{
		if (iPulse - to_ch->GetSafeboxLoadTime() < PASSES_PER_SEC(g_nPortalLimitTime))
		{
			to_ch->ChatPacket(CHAT_TYPE_INFO, "[LS;661;%d]", g_nPortalLimitTime);
			return;
		}

		if( true == to_ch->IsDead() )
		{
			return;
		}
	}

	sys_log(0, "CInputMain()::Exchange()  SubHeader %d ", pinfo->sub_header);

	if (iPulse - ch->GetSafeboxLoadTime() < PASSES_PER_SEC(g_nPortalLimitTime))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;661;%d]", g_nPortalLimitTime);
		return;
	}


	switch (pinfo->sub_header)
	{
		case EXCHANGE_SUBHEADER_CG_START:	// arg1 == vid of target character
			if (!ch->GetExchange())
			{
				if ((to_ch = CHARACTER_MANAGER::instance().Find(pinfo->arg1)))
				{
					//MONARCH_LIMIT
					/*
					if (to_ch->IsMonarch() || ch->IsMonarch())
					{
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("군주와는 거래를 할수가 없습니다"), g_nPortalLimitTime);
						return;
					}
					//END_MONARCH_LIMIT
					*/
					if (iPulse - ch->GetSafeboxLoadTime() < PASSES_PER_SEC(g_nPortalLimitTime))
					{
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot trade within %d seconds after opening the warehouse."), g_nPortalLimitTime);

						if (test_server)
							ch->ChatPacket(CHAT_TYPE_INFO, "[TestOnly][Safebox]Pulse %d LoadTime %d PASS %d", iPulse, ch->GetSafeboxLoadTime(), PASSES_PER_SEC(g_nPortalLimitTime));
						return;
					}

					if (iPulse - to_ch->GetSafeboxLoadTime() < PASSES_PER_SEC(g_nPortalLimitTime))
					{
						to_ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot trade within %d seconds after opening the warehouse."), g_nPortalLimitTime);


						if (test_server)
							to_ch->ChatPacket(CHAT_TYPE_INFO, "[TestOnly][Safebox]Pulse %d LoadTime %d PASS %d", iPulse, to_ch->GetSafeboxLoadTime(), PASSES_PER_SEC(g_nPortalLimitTime));
						return;
					}

#ifdef ENABLE_REMOVE_LIMIT_GOLD
					if (ch->GetGold() >= GOLD_MAX_MAX)
#else
					if (ch->GetGold() >= GOLD_MAX)
#endif
					{
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You have reached the yang limit."));

						sys_err("[OVERFLOG_GOLD] START (%lld) id %u name %s ", ch->GetGold(), ch->GetPlayerID(), ch->GetName());
						return;
					}

					if (to_ch->IsPC())
					{
						if (quest::CQuestManager::instance().GiveItemToPC(ch->GetPlayerID(), to_ch))
						{
							sys_log(0, "Exchange canceled by quest %s %s", ch->GetName(), to_ch->GetName());
							return;
						}
					}


					if (ch->GetMyShop() || ch->IsOpenSafebox() || ch->GetShopOwner() || ch->IsCubeOpen() || ch->IsAuraRefineWindowOpen())
					{
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot open a personal shop while other transactions are in progress."));
						return;
					}

					ch->ExchangeStart(to_ch);
				}
			}
			break;

		case EXCHANGE_SUBHEADER_CG_ITEM_ADD:	// arg1 == position of item, arg2 == position in exchange window
			if (ch->GetExchange())
			{
				if (ch->GetExchange()->GetCompany()->GetAcceptStatus() != true)
					ch->GetExchange()->AddItem(pinfo->Pos, pinfo->arg2);
			}
			break;

		case EXCHANGE_SUBHEADER_CG_ITEM_DEL:	// arg1 == position of item
			if (ch->GetExchange())
			{
				if (ch->GetExchange()->GetCompany()->GetAcceptStatus() != true)
					ch->GetExchange()->RemoveItem(pinfo->arg1);
			}
			break;

		case EXCHANGE_SUBHEADER_CG_ELK_ADD:	// arg1 == amount of gold
			if (ch->GetExchange())
			{
// #ifdef ENABLE_REMOVE_LIMIT_GOLD	
				// const unsigned long long nTotalGold = static_cast<unsigned long long>(ch->GetExchange()->GetCompany()->GetOwner()->GetGold()) + static_cast<unsigned long long>(pinfo->arg1);	
				// if (GOLD_MAX_MAX <= nTotalGold)
// #else
				// const int64_t nTotalGold = static_cast<int64_t>(ch->GetExchange()->GetCompany()->GetOwner()->GetGold()) + static_cast<int64_t>(pinfo->arg1);
				// if (GOLD_MAX <= nTotalGold)
// #endif
				// {
					// ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("상대방의 총금액이 20억 냥을 초과하여 거래를 할수가 없습니다.."));

// #ifdef ENABLE_REMOVE_LIMIT_GOLD	
					// sys_err("[OVERFLOW_GOLD] ELK_ADD (%llu) id %u name %s ",
// #else
					// sys_err("[OVERFLOW_GOLD] ELK_ADD (%u) id %u name %s ",
// #endif
							// ch->GetExchange()->GetCompany()->GetOwner()->GetGold(),
							// ch->GetExchange()->GetCompany()->GetOwner()->GetPlayerID(),
						   	// ch->GetExchange()->GetCompany()->GetOwner()->GetName());

					// return;
				// }

				if (ch->GetExchange()->GetCompany()->GetAcceptStatus() != true)
					ch->GetExchange()->AddGold(pinfo->arg1);
			}
			break;

		case EXCHANGE_SUBHEADER_CG_ACCEPT:	// arg1 == not used
			if (ch->GetExchange())
			{
				sys_log(0, "CInputMain()::Exchange() ==> ACCEPT ");
				ch->GetExchange()->Accept(true);
			}

			break;

		case EXCHANGE_SUBHEADER_CG_CANCEL:	// arg1 == not used
			if (ch->GetExchange())
				ch->GetExchange()->Cancel();
			break;
	}
}

void CInputMain::Position(LPCHARACTER ch, const char * data)
{
	struct command_position * pinfo = (struct command_position *) data;

	switch (pinfo->position)
	{
		case POSITION_GENERAL:
			ch->Standup();
			break;

		case POSITION_SITTING_CHAIR:
			ch->Sitdown(0);
			break;

		case POSITION_SITTING_GROUND:
			ch->Sitdown(1);
			break;
	}
}

static const int ComboSequenceBySkillLevel[3][8] =
{
	// 0   1   2   3   4   5   6   7
	{ 14, 15, 16, 17,  0,  0,  0,  0 },
	{ 14, 15, 16, 18, 20,  0,  0,  0 },
	{ 14, 15, 16, 18, 19, 17,  0,  0 },
};

#define COMBO_HACK_ALLOWABLE_MS	100

bool CheckComboHack(LPCHARACTER ch, BYTE bArg, DWORD dwTime, bool CheckSpeedHack)
{
	if(!gHackCheckEnable) return false;

	//	�װų� ���� ���¿����� ������ �� �����Ƿ�, skip�Ѵ�.
	//	�̷��� ���� ����, CHRACTER::CanMove()��
	//	if (IsStun() || IsDead()) return false;
	//	�� �߰��ϴ°� �´ٰ� �����ϳ�,
	//	�̹� �ٸ� �κп��� CanMove()�� IsStun(), IsDead()��
	//	���������� üũ�ϰ� �ֱ� ������ ������ ���� ������
	//	�ּ�ȭ�ϱ� ���� �̷��� ���� �ڵ带 ����´�.
	if (ch->IsStun() || ch->IsDead())
		return false;
	int ComboInterval = dwTime - ch->GetLastComboTime();
	int HackScalar = 0; // �⺻ ��Į�� ���� 1
#if 0
	sys_log(0, "COMBO: %s arg:%u seq:%u delta:%d checkspeedhack:%d",
			ch->GetName(), bArg, ch->GetComboSequence(), ComboInterval - ch->GetValidComboInterval(), CheckSpeedHack);
#endif
	// bArg 14 ~ 21�� ���� �� 8�޺� ����
	// 1. ù �޺�(14)�� ���� �ð� ���Ŀ� �ݺ� ����
	// 2. 15 ~ 21���� �ݺ� �Ұ���
	// 3. ���ʴ�� �����Ѵ�.
	if (bArg == 14)
	{
		if (CheckSpeedHack && ComboInterval > 0 && ComboInterval < ch->GetValidComboInterval() - COMBO_HACK_ALLOWABLE_MS)
		{
			// FIXME ù��° �޺��� �̻��ϰ� ���� �� ���� �־ 300���� ���� -_-;

			// �ټ��� ���Ϳ� ���� �ٿ�Ǵ� ��Ȳ���� ������ �ϸ�
			// ù��° �޺��� �ſ� ���� ���͹��� ������ ��Ȳ �߻�.
			// �̷� ���� �޺������� ƨ��� ��찡 �־� ���� �ڵ� �� Ȱ��ȭ.
			//HackScalar = 1 + (ch->GetValidComboInterval() - ComboInterval) / 300;

			//sys_log(0, "COMBO_HACK: 2 %s arg:%u interval:%d valid:%u atkspd:%u riding:%s",
			//		ch->GetName(),
			//		bArg,
			//		ComboInterval,
			//		ch->GetValidComboInterval(),
			//		ch->GetPoint(POINT_ATT_SPEED),
			//	    ch->IsRiding() ? "yes" : "no");
		}

		ch->SetComboSequence(1);
		ch->SetValidComboInterval((int) (ani_combo_speed(ch, 1) / (ch->GetPoint(POINT_ATT_SPEED) / 100.f)));
		ch->SetLastComboTime(dwTime);
	}
	else if (bArg > 14 && bArg < 22)
	{
		int idx = MIN(2, ch->GetComboIndex());

		if (ch->GetComboSequence() > 5) // ���� 6�޺� �̻��� ����.
		{
			HackScalar = 1;
			ch->SetValidComboInterval(300);
			sys_log(0, "COMBO_HACK: 5 %s combo_seq:%d", ch->GetName(), ch->GetComboSequence());
		}
		// �ڰ� �ּ� �޺� ����ó��
		else if (bArg == 21 &&
				 idx == 2 &&
				 ch->GetComboSequence() == 5 &&
				 ch->GetJob() == JOB_ASSASSIN &&
				 ch->GetWear(WEAR_WEAPON) &&
				 ch->GetWear(WEAR_WEAPON)->GetSubType() == WEAPON_DAGGER)
			ch->SetValidComboInterval(300);
		else if (ComboSequenceBySkillLevel[idx][ch->GetComboSequence()] != bArg)
		{
			HackScalar = 1;
			ch->SetValidComboInterval(300);

			sys_log(0, "COMBO_HACK: 3 %s arg:%u valid:%u combo_idx:%d combo_seq:%d",
					ch->GetName(),
					bArg,
					ComboSequenceBySkillLevel[idx][ch->GetComboSequence()],
					idx,
					ch->GetComboSequence());
		}
		else
		{
			if (CheckSpeedHack && ComboInterval < ch->GetValidComboInterval() - COMBO_HACK_ALLOWABLE_MS)
			{
				HackScalar = 1 + (ch->GetValidComboInterval() - ComboInterval) / 100;

				sys_log(0, "COMBO_HACK: 2 %s arg:%u interval:%d valid:%u atkspd:%u riding:%s",
						ch->GetName(),
						bArg,
						ComboInterval,
						ch->GetValidComboInterval(),
						ch->GetPoint(POINT_ATT_SPEED),
						ch->IsRiding() ? "yes" : "no");
			}

			// ���� ���� ���� 15�� ~ 16���� �ݺ��Ѵ�
			//if (ch->IsHorseRiding())
			if (ch->IsRiding())
				ch->SetComboSequence(ch->GetComboSequence() == 1 ? 2 : 1);
			else
				ch->SetComboSequence(ch->GetComboSequence() + 1);

			ch->SetValidComboInterval((int) (ani_combo_speed(ch, bArg - 13) / (ch->GetPoint(POINT_ATT_SPEED) / 100.f)));
			ch->SetLastComboTime(dwTime);
		}
	}
	else if (bArg == 13) // �⺻ ���� (�а�(Polymorph)���� �� �´�)
	{
		if (CheckSpeedHack && ComboInterval > 0 && ComboInterval < ch->GetValidComboInterval() - COMBO_HACK_ALLOWABLE_MS)
		{
			// �ټ��� ���Ϳ� ���� �ٿ�Ǵ� ��Ȳ���� ������ �ϸ�
			// ù��° �޺��� �ſ� ���� ���͹��� ������ ��Ȳ �߻�.
			// �̷� ���� �޺������� ƨ��� ��찡 �־� ���� �ڵ� �� Ȱ��ȭ.
			//HackScalar = 1 + (ch->GetValidComboInterval() - ComboInterval) / 100;

			//sys_log(0, "COMBO_HACK: 6 %s arg:%u interval:%d valid:%u atkspd:%u",
			//		ch->GetName(),
			//		bArg,
			//		ComboInterval,
			//		ch->GetValidComboInterval(),
			//		ch->GetPoint(POINT_ATT_SPEED));
		}

		if (ch->GetRaceNum() >= MAIN_RACE_MAX_NUM)
		{
			// POLYMORPH_BUG_FIX

			// DELETEME
			/*
			const CMotion * pkMotion = CMotionManager::instance().GetMotion(ch->GetRaceNum(), MAKE_MOTION_KEY(MOTION_MODE_GENERAL, MOTION_NORMAL_ATTACK));

			if (!pkMotion)
				sys_err("cannot find motion by race %u", ch->GetRaceNum());
			else
			{
				// ������ ����̶�� 1000.f�� ���ؾ� ������ Ŭ���̾�Ʈ�� �ִϸ��̼� �ӵ��� 90%����
				// ���� �ִϸ��̼� �������� ����ϹǷ� 900.f�� ���Ѵ�.
				int k = (int) (pkMotion->GetDuration() / ((float) ch->GetPoint(POINT_ATT_SPEED) / 100.f) * 900.f);
				ch->SetValidComboInterval(k);
				ch->SetLastComboTime(dwTime);
			}
			*/
			float normalAttackDuration = CMotionManager::instance().GetNormalAttackDuration(ch->GetRaceNum());
			int k = (int) (normalAttackDuration / ((float) ch->GetPoint(POINT_ATT_SPEED) / 100.f) * 900.f);
			ch->SetValidComboInterval(k);
			ch->SetLastComboTime(dwTime);
			// END_OF_POLYMORPH_BUG_FIX
		}
		else
		{
			// ���� �ȵǴ� �޺��� �Դ� ��Ŀ�� ���ɼ�?
			//if (ch->GetDesc()->DelayedDisconnect(number(2, 9)))
			//{
			//	LogManager::instance().HackLog("Hacker", ch);
			//	sys_log(0, "HACKER: %s arg %u", ch->GetName(), bArg);
			//}

			// �� �ڵ�� ����, ���������� Ǫ�� �߿� ���� �ϸ�,
			// ���� ������ �ν��ϴ� ��찡 �ִ�.

			// �ڼ��� ������,
			// �������� poly 0�� ó��������,
			// Ŭ�󿡼� �� ��Ŷ�� �ޱ� ����, ���� ����. <- ��, ���� ���¿��� ����.
			//
			// �׷��� Ŭ�󿡼��� ������ �� ���·� �����ߴٴ� Ŀ�ǵ带 ������ (arg == 13)
			//
			// ���������� race�� �ΰ��ε� �������´� ���� ���̴�! ��� �Ͽ� ��üũ�� �ߴ�.

			// ��� ���� ���Ͽ� ���� ���� Ŭ���̾�Ʈ���� �Ǵ��ؼ� ���� ���� �ƴ϶�,
			// �������� �Ǵ��ؾ� �� ���ε�... �� �̷��� �س�����...
			// by rtsummit
		}
	}
	else
	{
		// ���� �ȵǴ� �޺��� �Դ� ��Ŀ�� ���ɼ�?
		if (ch->GetDesc()->DelayedDisconnect(number(2, 9)))
		{
			LogManager::instance().HackLog("Hacker", ch);
			sys_log(0, "HACKER: %s arg %u", ch->GetName(), bArg);
		}

		HackScalar = 10;
		ch->SetValidComboInterval(300);
	}

	if (HackScalar)
	{
		// ���� Ÿ�ų� ������ �� 1.5�ʰ� ������ ������ �������� �ʵ� ���ݷ��� ���� �ϴ� ó��
		if (get_dword_time() - ch->GetLastMountTime() > 1500)
			ch->IncreaseComboHackCount(1 + HackScalar);

		ch->SkipComboAttackByTime(ch->GetValidComboInterval());
	}

	return HackScalar;


}

void CInputMain::Move(LPCHARACTER ch, const char * data)
{
	if (!ch->CanMove())
		return;

	struct command_move * pinfo = (struct command_move *) data;

	if (pinfo->bFunc >= FUNC_MAX_NUM && !(pinfo->bFunc & 0x80))
	{
		sys_err("invalid move type: %s", ch->GetName());
		return;
	}

	//enum EMoveFuncType
	//{
	//	FUNC_WAIT,
	//	FUNC_MOVE,
	//	FUNC_ATTACK,
	//	FUNC_COMBO,
	//	FUNC_MOB_SKILL,
	//	_FUNC_SKILL,
	//	FUNC_MAX_NUM,
	//	FUNC_SKILL = 0x80,
	//};

	// �ڷ���Ʈ �� üũ

//	if (!test_server)
	{
		const float fDist = DISTANCE_SQRT((ch->GetX() - pinfo->lX) / 100, (ch->GetY() - pinfo->lY) / 100);
		// @fixme106 (changed 40 to 60)
		if (((false == ch->IsRiding() && fDist > 750) || fDist > 999) && OXEVENT_MAP_INDEX != ch->GetMapIndex())
		{
#ifdef ENABLE_HACK_TELEPORT_LOG // @warme006
			{
				const PIXEL_POSITION & warpPos = ch->GetWarpPosition();

				if (warpPos.x == 0 && warpPos.y == 0)
					LogManager::instance().HackLog("Teleport", ch); // ����Ȯ�� �� ����
			}
#endif
			sys_log(0, "MOVE: %s trying to move too far (dist: %.1fm) Riding(%d)", ch->GetName(), fDist, ch->IsRiding());

			ch->Show(ch->GetMapIndex(), ch->GetX(), ch->GetY(), ch->GetZ());
			ch->Stop();
			return;
		}
#ifdef ENABLE_CHECK_GHOSTMODE
		if (ch->IsPC() && ch->IsDead())
		{
			sys_log(0, "MOVE: %s trying to move as dead", ch->GetName());

			ch->Show(ch->GetMapIndex(), ch->GetX(), ch->GetY(), ch->GetZ());
			ch->Stop();
			return;
		}
#endif
		//
		// ���ǵ���(SPEEDHACK) Check
		//
		DWORD dwCurTime = get_dword_time();
		// �ð��� Sync�ϰ� 7�� �� ���� �˻��Ѵ�. (20090702 ������ 5�ʿ���)
		bool CheckSpeedHack = (false == ch->GetDesc()->IsHandshaking() && dwCurTime - ch->GetDesc()->GetClientTime() > 7000);

		if (pinfo->bFunc == FUNC_COMBO && g_bCheckMultiHack)
		{
			CheckComboHack(ch, pinfo->bArg, pinfo->dwTime, CheckSpeedHack); // �޺� üũ
		}
	}

	if (pinfo->bFunc == FUNC_MOVE)
	{
		if (ch->GetLimitPoint(POINT_MOV_SPEED) == 0)
			return;

		ch->SetRotation(pinfo->bRot * 5);	// �ߺ� �ڵ�
		ch->ResetStopTime();				// ""

		ch->Goto(pinfo->lX, pinfo->lY);
	}
	else
	{
		if (pinfo->bFunc == FUNC_ATTACK || pinfo->bFunc == FUNC_COMBO)
			ch->OnMove(true);
		else if (pinfo->bFunc & FUNC_SKILL)
		{
			const int MASK_SKILL_MOTION = 0x7F;
			unsigned int motion = pinfo->bFunc & MASK_SKILL_MOTION;

			if (!ch->IsUsableSkillMotion(motion))
			{
				const char* name = ch->GetName();
				unsigned int job = ch->GetJob();
				unsigned int group = ch->GetSkillGroup();

				char szBuf[256];
				snprintf(szBuf, sizeof(szBuf), "SKILL_HACK: name=%s, job=%d, group=%d, motion=%d", name, job, group, motion);
				LogManager::instance().HackLog(szBuf, ch->GetDesc()->GetAccountTable().login, ch->GetName(), ch->GetDesc()->GetHostName());
				sys_log(0, "%s", szBuf);

				if (test_server)
				{
					ch->GetDesc()->DelayedDisconnect(number(2, 8));
					ch->ChatPacket(CHAT_TYPE_INFO, szBuf);
				}
				else
				{
					ch->GetDesc()->DelayedDisconnect(number(150, 500));
				}
			}

			ch->OnMove();
		}

		ch->SetRotation(pinfo->bRot * 5);	// �ߺ� �ڵ�
		ch->ResetStopTime();				// ""

		ch->Move(pinfo->lX, pinfo->lY);
		ch->Stop();
		ch->StopStaminaConsume();
	}

	TPacketGCMove pack;

	pack.bHeader      = HEADER_GC_MOVE;
	pack.bFunc        = pinfo->bFunc;
	pack.bArg         = pinfo->bArg;
	pack.bRot         = pinfo->bRot;
	pack.dwVID        = ch->GetVID();
	pack.lX           = pinfo->lX;
	pack.lY           = pinfo->lY;
	pack.dwTime       = pinfo->dwTime;
	pack.dwDuration   = (pinfo->bFunc == FUNC_MOVE) ? ch->GetCurrentMoveDuration() : 0;

	ch->PacketAround(&pack, sizeof(TPacketGCMove), ch);
/*
	if (pinfo->dwTime == 10653691) // ����� �߰�
	{
		if (ch->GetDesc()->DelayedDisconnect(number(15, 30)))
			LogManager::instance().HackLog("Debugger", ch);

	}
	else if (pinfo->dwTime == 10653971) // Softice �߰�
	{
		if (ch->GetDesc()->DelayedDisconnect(number(15, 30)))
			LogManager::instance().HackLog("Softice", ch);
	}
*/
	/*
	sys_log(0,
			"MOVE: %s Func:%u Arg:%u Pos:%dx%d Time:%u Dist:%.1f",
			ch->GetName(),
			pinfo->bFunc,
			pinfo->bArg,
			pinfo->lX / 100,
			pinfo->lY / 100,
			pinfo->dwTime,
			fDist);
	*/
}

void CInputMain::Attack(LPCHARACTER ch, const BYTE header, const char* data)
{
	if (NULL == ch)
		return;

	struct type_identifier
	{
		BYTE header;
		BYTE type;
	};

	const struct type_identifier* const type = reinterpret_cast<const struct type_identifier*>(data);

	if (type->type > 0)
	{
		if (false == ch->CanUseSkill(type->type))
		{
			return;
		}

		switch (type->type)
		{
			case SKILL_GEOMPUNG:
			case SKILL_SANGONG:
			case SKILL_YEONSA:
			case SKILL_KWANKYEOK:
			case SKILL_HWAJO:
			case SKILL_GIGUNG:
			case SKILL_PABEOB:
			case SKILL_MARYUNG:
			case SKILL_TUSOK:
			case SKILL_MAHWAN:
			case SKILL_BIPABU:
			case SKILL_NOEJEON:
			case SKILL_CHAIN:
			case SKILL_HORSE_WILDATTACK_RANGE:
				if (HEADER_CG_SHOOT != type->header)
				{
					if (test_server)
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Attack :name[%s] Vnum[%d] can't use skill by attack(warning)"), type->type);
					return;
				}
				break;
		}
	}

	switch (header)
	{
		case HEADER_CG_ATTACK:
			{
				if (NULL == ch->GetDesc())
					return;

				const TPacketCGAttack* const packMelee = reinterpret_cast<const TPacketCGAttack*>(data);

				ch->GetDesc()->AssembleCRCMagicCube(packMelee->bCRCMagicCubeProcPiece, packMelee->bCRCMagicCubeFilePiece);

				LPCHARACTER	victim = CHARACTER_MANAGER::instance().Find(packMelee->dwVID);

				if (NULL == victim || ch == victim)
					return;

				switch (victim->GetCharType())
				{
					case CHAR_TYPE_NPC:
					case CHAR_TYPE_WARP:
					case CHAR_TYPE_GOTO:
						return;
				}

				if (packMelee->bType > 0)
				{
					if (false == ch->CheckSkillHitCount(packMelee->bType, victim->GetVID()))
					{
						return;
					}
				}

				ch->Attack(victim, packMelee->bType);
			}
			break;

		case HEADER_CG_SHOOT:
			{
				const TPacketCGShoot* const packShoot = reinterpret_cast<const TPacketCGShoot*>(data);

				ch->Shoot(packShoot->bType);
			}
			break;
	}
}

int CInputMain::SyncPosition(LPCHARACTER ch, const char * c_pcData, size_t uiBytes)
{
	const TPacketCGSyncPosition* pinfo = reinterpret_cast<const TPacketCGSyncPosition*>( c_pcData );

	if (uiBytes < pinfo->wSize)
		return -1;

	int iExtraLen = pinfo->wSize - sizeof(TPacketCGSyncPosition);

	if (iExtraLen < 0)
	{
		sys_err("invalid packet length (len %d size %u buffer %u)", iExtraLen, pinfo->wSize, uiBytes);
		ch->GetDesc()->SetPhase(PHASE_CLOSE);
		return -1;
	}

	if (0 != (iExtraLen % sizeof(TPacketCGSyncPositionElement)))
	{
		sys_err("invalid packet length %d (name: %s)", pinfo->wSize, ch->GetName());
		return iExtraLen;
	}

	int iCount = iExtraLen / sizeof(TPacketCGSyncPositionElement);

	if (iCount <= 0)
		return iExtraLen;

	static const int nCountLimit = 16;

	if( iCount > nCountLimit )
	{
		//LogManager::instance().HackLog( "SYNC_POSITION_HACK", ch );
		sys_err( "Too many SyncPosition Count(%d) from Name(%s)", iCount, ch->GetName() );
		//ch->GetDesc()->SetPhase(PHASE_CLOSE);
		//return -1;
		iCount = nCountLimit;
	}

	TEMP_BUFFER tbuf;
	LPBUFFER lpBuf = tbuf.getptr();

	TPacketGCSyncPosition * pHeader = (TPacketGCSyncPosition *) buffer_write_peek(lpBuf);
	buffer_write_proceed(lpBuf, sizeof(TPacketGCSyncPosition));

	const TPacketCGSyncPositionElement* e =
		reinterpret_cast<const TPacketCGSyncPositionElement*>(c_pcData + sizeof(TPacketCGSyncPosition));

	timeval tvCurTime;
	gettimeofday(&tvCurTime, NULL);

	for (int i = 0; i < iCount; ++i, ++e)
	{
		LPCHARACTER victim = CHARACTER_MANAGER::instance().Find(e->dwVID);

		if (!victim)
			continue;

		switch (victim->GetCharType())
		{
			case CHAR_TYPE_NPC:
			case CHAR_TYPE_WARP:
			case CHAR_TYPE_GOTO:
				continue;
		}

		// ������ �˻�
		if (!victim->SetSyncOwner(ch))
			continue;

		const float fDistWithSyncOwner = DISTANCE_SQRT( (victim->GetX() - ch->GetX()) / 100, (victim->GetY() - ch->GetY()) / 100 );
		static const float fLimitDistWithSyncOwner = 2500.f + 1000.f;
		// victim���� �Ÿ��� 2500 + a �̻��̸� ������ ����.
		//	�Ÿ� ���� : Ŭ���̾�Ʈ�� __GetSkillTargetRange, __GetBowRange �Լ�
		//	2500 : ��ų proto���� ���� ��Ÿ��� �� ��ų�� ��Ÿ�, �Ǵ� Ȱ�� ��Ÿ�
		//	a = POINT_BOW_DISTANCE ��... �ε� ������ ����ϴ� �������� �� �𸣰���. �������̳� ����, ��ų, ����Ʈ���� ���µ�...
		//		�׷��� Ȥ�ó� �ϴ� ������ ���۷� ����� ���ؼ� 1000.f �� ��...
		if (fDistWithSyncOwner > fLimitDistWithSyncOwner)
		{
			// g_iSyncHackLimitCount�� ������ ����.
			if (ch->GetSyncHackCount() < g_iSyncHackLimitCount)
			{
				ch->SetSyncHackCount(ch->GetSyncHackCount() + 1);
				continue;
			}
			else
			{
				LogManager::instance().HackLog( "SYNC_POSITION_HACK", ch );

				sys_err( "Too far SyncPosition DistanceWithSyncOwner(%f)(%s) from Name(%s) CH(%d,%d) VICTIM(%d,%d) SYNC(%d,%d)",
					fDistWithSyncOwner, victim->GetName(), ch->GetName(), ch->GetX(), ch->GetY(), victim->GetX(), victim->GetY(),
					e->lX, e->lY );

				ch->GetDesc()->SetPhase(PHASE_CLOSE);

				return -1;
			}
		}

		const float fDist = DISTANCE_SQRT( (victim->GetX() - e->lX) / 100, (victim->GetY() - e->lY) / 100 );
		static const long g_lValidSyncInterval = 100 * 1000; // 100ms
		const timeval &tvLastSyncTime = victim->GetLastSyncTime();
		timeval *tvDiff = timediff(&tvCurTime, &tvLastSyncTime);

		// SyncPosition�� �ǿ��Ͽ� Ÿ������ �̻��� ������ ������ �� ����ϱ� ���Ͽ�,
		// ���� ������ g_lValidSyncInterval ms �̳��� �ٽ� SyncPosition�Ϸ��� �ϸ� ������ ����.
		if (tvDiff->tv_sec == 0 && tvDiff->tv_usec < g_lValidSyncInterval)
		{
			// g_iSyncHackLimitCount�� ������ ����.
			if (ch->GetSyncHackCount() < g_iSyncHackLimitCount)
			{
				ch->SetSyncHackCount(ch->GetSyncHackCount() + 1);
				continue;
			}
			else
			{
				LogManager::instance().HackLog( "SYNC_POSITION_HACK", ch );

				sys_err( "Too often SyncPosition Interval(%ldms)(%s) from Name(%s) VICTIM(%d,%d) SYNC(%d,%d)",
					tvDiff->tv_sec * 1000 + tvDiff->tv_usec / 1000, victim->GetName(), ch->GetName(), victim->GetX(), victim->GetY(),
					e->lX, e->lY );

				ch->GetDesc()->SetPhase(PHASE_CLOSE);

				return -1;
			}
		}
		else if( fDist > 25.0f )
		{
			LogManager::instance().HackLog( "SYNC_POSITION_HACK", ch );

			sys_err( "Too far SyncPosition Distance(%f)(%s) from Name(%s) CH(%d,%d) VICTIM(%d,%d) SYNC(%d,%d)",
				   	fDist, victim->GetName(), ch->GetName(), ch->GetX(), ch->GetY(), victim->GetX(), victim->GetY(),
				  e->lX, e->lY );

			ch->GetDesc()->SetPhase(PHASE_CLOSE);

			return -1;
		}
		else
		{
			victim->SetLastSyncTime(tvCurTime);
			victim->Sync(e->lX, e->lY);
			buffer_write(lpBuf, e, sizeof(TPacketCGSyncPositionElement));
		}
	}

	if (buffer_size(lpBuf) != sizeof(TPacketGCSyncPosition))
	{
		pHeader->bHeader = HEADER_GC_SYNC_POSITION;
		pHeader->wSize = buffer_size(lpBuf);

		ch->PacketAround(buffer_read_peek(lpBuf), buffer_size(lpBuf), ch);
	}

	return iExtraLen;
}

void CInputMain::FlyTarget(LPCHARACTER ch, const char * pcData, BYTE bHeader)
{
	TPacketCGFlyTargeting * p = (TPacketCGFlyTargeting *) pcData;
	ch->FlyTarget(p->dwTargetVID, p->x, p->y, bHeader);
}

void CInputMain::UseSkill(LPCHARACTER ch, const char * pcData)
{
	TPacketCGUseSkill * p = (TPacketCGUseSkill *) pcData;
	ch->UseSkill(p->dwVnum, CHARACTER_MANAGER::instance().Find(p->dwVID));
}

void CInputMain::ScriptButton(LPCHARACTER ch, const void* c_pData)
{
	TPacketCGScriptButton * p = (TPacketCGScriptButton *) c_pData;
	sys_log(0, "QUEST ScriptButton pid %d idx %u", ch->GetPlayerID(), p->idx);

	quest::PC* pc = quest::CQuestManager::instance().GetPCForce(ch->GetPlayerID());
	if (pc && pc->IsConfirmWait())
	{
		quest::CQuestManager::instance().Confirm(ch->GetPlayerID(), quest::CONFIRM_TIMEOUT);
	}
	else if (p->idx & 0x80000000)
	{
		//����Ʈ â���� Ŭ����(__SelectQuest) �����
		quest::CQuestManager::Instance().QuestInfo(ch->GetPlayerID(), p->idx & 0x7fffffff);
	}
	else
	{
		quest::CQuestManager::Instance().QuestButton(ch->GetPlayerID(), p->idx);
	}
}

void CInputMain::ScriptAnswer(LPCHARACTER ch, const void* c_pData)
{
	TPacketCGScriptAnswer * p = (TPacketCGScriptAnswer *) c_pData;
	sys_log(0, "QUEST ScriptAnswer pid %d answer %d", ch->GetPlayerID(), p->answer);

	if (p->answer > 250) // ���� ��ư�� ���� �������� �� ��Ŷ�� ���
	{
		quest::CQuestManager::Instance().Resume(ch->GetPlayerID());
	}
	else // ���� ��ư�� ��� �� ��Ŷ�� ���
	{
		quest::CQuestManager::Instance().Select(ch->GetPlayerID(),  p->answer);
	}
}


// SCRIPT_SELECT_ITEM
void CInputMain::ScriptSelectItem(LPCHARACTER ch, const void* c_pData)
{
	TPacketCGScriptSelectItem* p = (TPacketCGScriptSelectItem*) c_pData;
	sys_log(0, "QUEST ScriptSelectItem pid %d answer %d", ch->GetPlayerID(), p->selection);
	quest::CQuestManager::Instance().SelectItem(ch->GetPlayerID(), p->selection);
}
// END_OF_SCRIPT_SELECT_ITEM

void CInputMain::QuestInputString(LPCHARACTER ch, const void* c_pData)
{
	TPacketCGQuestInputString * p = (TPacketCGQuestInputString*) c_pData;

	char msg[65];
	strlcpy(msg, p->msg, sizeof(msg));
	sys_log(0, "QUEST InputString pid %u msg %s", ch->GetPlayerID(), msg);

	quest::CQuestManager::Instance().Input(ch->GetPlayerID(), msg);
}

void CInputMain::QuestConfirm(LPCHARACTER ch, const void* c_pData)
{
	TPacketCGQuestConfirm* p = (TPacketCGQuestConfirm*) c_pData;
	LPCHARACTER ch_wait = CHARACTER_MANAGER::instance().FindByPID(p->requestPID);
	if (p->answer)
		p->answer = quest::CONFIRM_YES;
	sys_log(0, "QuestConfirm from %s pid %u name %s answer %d", ch->GetName(), p->requestPID, (ch_wait)?ch_wait->GetName():"", p->answer);
	if (ch_wait)
	{
		quest::CQuestManager::Instance().Confirm(ch_wait->GetPlayerID(), (quest::EQuestConfirmType) p->answer, ch->GetPlayerID());
	}
}

void CInputMain::Target(LPCHARACTER ch, const char * pcData)
{
	TPacketCGTarget * p = (TPacketCGTarget *) pcData;

	building::LPOBJECT pkObj = building::CManager::instance().FindObjectByVID(p->dwVID);

	if (pkObj)
	{
		TPacketGCTarget pckTarget;
		pckTarget.header = HEADER_GC_TARGET;
		pckTarget.dwVID = p->dwVID;
#if defined(__SHIP_DEFENSE__)
		pckTarget.bAlliance = false;
		pckTarget.iAllianceMinHP = 0;
		pckTarget.iAllianceMaxHP = 0;
#endif
		ch->GetDesc()->Packet(&pckTarget, sizeof(TPacketGCTarget));
	}
	else
		ch->SetTarget(CHARACTER_MANAGER::instance().Find(p->dwVID));
}

void CInputMain::Warp(LPCHARACTER ch, const char * pcData)
{
	ch->WarpEnd();
}

void CInputMain::SafeboxCheckin(LPCHARACTER ch, const char * c_pData)
{
	if (quest::CQuestManager::instance().GetPCForce(ch->GetPlayerID())->IsRunning() == true)
		return;

	TPacketCGSafeboxCheckin * p = (TPacketCGSafeboxCheckin *) c_pData;

	if (!ch->CanHandleItem())
		return;

	CSafebox * pkSafebox = ch->GetSafebox();
	LPITEM pkItem = ch->GetItem(p->ItemPos);

	if (!pkSafebox || !pkItem)
		return;

#ifdef ENABLE_CUSTOM_INVENTORY
	if (IS_SET(pkItem->GetFlag(), ITEM_FLAG_IRREMOVABLE))
	{
		 if((pkItem->GetCell() >= ch->Inventory_Size() && pkItem->GetCell() < CUSTOM_INVENTORY_SLOT_START) || pkItem->GetCell() >= CUSTOM_INVENTORY_SLOT_END)
		 {
	    	ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("@@tradus(input_main.cpp)dracu mai stie ce trebuia sa fie aici."));
			return; 
		 }
	}	
#else
	if (pkItem->GetCell() >= INVENTORY_MAX_NUM && IS_SET(pkItem->GetFlag(), ITEM_FLAG_IRREMOVABLE))
	{
	    ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<â��> â���� �ű� �� ���� ������ �Դϴ�."));
	    return;
	}

#endif

	if (!pkSafebox->IsEmpty(p->bSafePos, pkItem->GetSize()))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;666]");
		return;
	}

	if (pkItem->GetVnum() == UNIQUE_ITEM_SAFEBOX_EXPAND)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;667]");
		return;
	}

	if( IS_SET(pkItem->GetAntiFlag(), ITEM_ANTIFLAG_SAFEBOX) )
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;667]");
		return;
	}

	if (true == pkItem->isLocked())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;667]");
		return;
	}

#ifdef ENABLE_WEAPON_COSTUME_SYSTEM
	if (pkItem->IsEquipped())
	{
		int iWearCell = pkItem->FindEquipCell(ch);
		if (iWearCell == WEAR_WEAPON)
		{
			LPITEM costumeWeapon = ch->GetWear(WEAR_COSTUME_WEAPON);
			if (costumeWeapon && !ch->UnequipItem(costumeWeapon))
			{
				ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot unequip the costume weapon because there is not enough space"));
				return;
			}
		}
	}
#endif

	// @fixme140 BEGIN
	if (ITEM_BELT == pkItem->GetType() && CBeltInventoryHelper::IsExistItemInBeltInventory(ch))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1095]");
		return;
	}
	// @fixme140 END

	pkItem->RemoveFromCharacter();
	if (!pkItem->IsDragonSoul())
		ch->SyncQuickslot(QUICKSLOT_TYPE_ITEM, p->ItemPos.cell, 255);
	pkSafebox->Add(p->bSafePos, pkItem);

	char szHint[128];
	snprintf(szHint, sizeof(szHint), "%s %u", pkItem->GetName(), pkItem->GetCount());
	LogManager::instance().ItemLog(ch, pkItem, "SAFEBOX PUT", szHint);
}

void CInputMain::SafeboxCheckout(LPCHARACTER ch, const char * c_pData, bool bMall)
{
	TPacketCGSafeboxCheckout * p = (TPacketCGSafeboxCheckout *) c_pData;

	if (!ch->CanHandleItem())
		return;

	CSafebox * pkSafebox;

	if (bMall)
		pkSafebox = ch->GetMall();
	else
		pkSafebox = ch->GetSafebox();

	if (!pkSafebox)
		return;

	LPITEM pkItem = pkSafebox->Get(p->bSafePos);

	if (!pkItem)
		return;
	
#ifdef ENABLE_CUSTOM_INVENTORY
	TItemPos realPos = p->ItemPos;
	if (realPos.IsCustomInventoryPosition() && realPos.GetCustomInventoryCategory() != pkItem->GetItemCategory())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Nu poti plasa acest obiect aici."));
		return;
	}
#endif

	if (!ch->IsEmptyItemGrid(p->ItemPos, pkItem->GetSize()))
		return;

	if (pkItem->IsDragonSoul())
	{
		if (bMall)
		{
			DSManager::instance().DragonSoulItemInitialize(pkItem);
		}

		if (DRAGON_SOUL_INVENTORY != p->ItemPos.window_type)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;666]");
			return;
		}

		TItemPos DestPos = p->ItemPos;
		if (!DSManager::instance().IsValidCellForThisItem(pkItem, DestPos))
		{
			int iCell = ch->GetEmptyDragonSoulInventory(pkItem);
			if (iCell < 0)
			{
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;666]");
				return ;
			}
			DestPos = TItemPos (DRAGON_SOUL_INVENTORY, iCell);
		}

		pkSafebox->Remove(p->bSafePos);
		pkItem->AddToCharacter(ch, DestPos);
		ITEM_MANAGER::instance().FlushDelayedSave(pkItem);
	}
	else
	{
		if (DRAGON_SOUL_INVENTORY == p->ItemPos.window_type)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;666]");
			return;
		}
		// @fixme119
		if (p->ItemPos.IsBeltInventoryPosition() && false == CBeltInventoryHelper::CanMoveIntoBeltInventory(pkItem))
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1097]");
			return;
		}

		pkSafebox->Remove(p->bSafePos);
		pkItem->AddToCharacter(ch, p->ItemPos);
		ITEM_MANAGER::instance().FlushDelayedSave(pkItem);
	}

	DWORD dwID = pkItem->GetID();
	db_clientdesc->DBPacketHeader(HEADER_GD_ITEM_FLUSH, 0, sizeof(DWORD));
	db_clientdesc->Packet(&dwID, sizeof(DWORD));

	char szHint[128];
	snprintf(szHint, sizeof(szHint), "%s %u", pkItem->GetName(), pkItem->GetCount());
	if (bMall)
		LogManager::instance().ItemLog(ch, pkItem, "MALL GET", szHint);
	else
		LogManager::instance().ItemLog(ch, pkItem, "SAFEBOX GET", szHint);
}

void CInputMain::SafeboxItemMove(LPCHARACTER ch, const char * data)
{
	struct command_item_move * pinfo = (struct command_item_move *) data;

	if (!ch->CanHandleItem())
		return;

	if (!ch->GetSafebox())
		return;

	ch->GetSafebox()->MoveItem(pinfo->Cell.cell, pinfo->CellTo.cell, pinfo->count);
}

// PARTY_JOIN_BUG_FIX
void CInputMain::PartyInvite(LPCHARACTER ch, const char * c_pData)
{
	if (ch->GetArena())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
		return;
	}

	TPacketCGPartyInvite * p = (TPacketCGPartyInvite*) c_pData;

	LPCHARACTER pInvitee = CHARACTER_MANAGER::instance().Find(p->vid);

	if (!pInvitee || !ch->GetDesc() || !pInvitee->GetDesc())
	{
		sys_err("PARTY Cannot find invited character");
		return;
	}

	ch->PartyInvite(pInvitee);
}

void CInputMain::PartyInviteAnswer(LPCHARACTER ch, const char * c_pData)
{
	if (ch->GetArena())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
		return;
	}

	TPacketCGPartyInviteAnswer * p = (TPacketCGPartyInviteAnswer*) c_pData;

	LPCHARACTER pInviter = CHARACTER_MANAGER::instance().Find(p->leader_vid);

	// pInviter 가 ch 에게 파티 요청을 했었다.

	if (!pInviter)
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;668]");
	else if (!p->accept)
		pInviter->PartyInviteDeny(ch->GetPlayerID());
	else
		pInviter->PartyInviteAccept(ch);
}
// END_OF_PARTY_JOIN_BUG_FIX

void CInputMain::PartySetState(LPCHARACTER ch, const char* c_pData)
{
	if (!CPartyManager::instance().IsEnablePCParty())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;530]");
		return;
	}

	TPacketCGPartySetState* p = (TPacketCGPartySetState*) c_pData;

	if (!ch->GetParty())
		return;

	if (ch->GetParty()->GetLeaderPID() != ch->GetPlayerID())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;671]");
		return;
	}

	if (!ch->GetParty()->IsMember(p->pid))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;672]");
		return;
	}

	DWORD pid = p->pid;
	sys_log(0, "PARTY SetRole pid %d to role %d state %s", pid, p->byRole, p->flag ? "on" : "off");

	switch (p->byRole)
	{
		case PARTY_ROLE_NORMAL:
			break;

		case PARTY_ROLE_ATTACKER:
		case PARTY_ROLE_TANKER:
		case PARTY_ROLE_BUFFER:
		case PARTY_ROLE_SKILL_MASTER:
		case PARTY_ROLE_HASTE:
		case PARTY_ROLE_DEFENDER:
			if (ch->GetParty()->SetRole(pid, p->byRole, p->flag))
			{
				TPacketPartyStateChange pack;
				pack.dwLeaderPID = ch->GetPlayerID();
				pack.dwPID = p->pid;
				pack.bRole = p->byRole;
				pack.bFlag = p->flag;
				db_clientdesc->DBPacket(HEADER_GD_PARTY_STATE_CHANGE, 0, &pack, sizeof(pack));
			}
			/* else
			   ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<파티> 어태커 설정에 실패하였습니다.")); */
			break;

		default:
			sys_err("wrong byRole in PartySetState Packet name %s state %d", ch->GetName(), p->byRole);
			break;
	}
}

void CInputMain::PartyRemove(LPCHARACTER ch, const char* c_pData)
{
	if (ch->GetArena())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;403]");
		return;
	}

	if (!CPartyManager::instance().IsEnablePCParty())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;530]");
		return;
	}

	if (ch->GetDungeon())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;673]");
		return;
	}

	TPacketCGPartyRemove* p = (TPacketCGPartyRemove*) c_pData;

	if (!ch->GetParty())
		return;

	LPPARTY pParty = ch->GetParty();
	if (pParty->GetLeaderPID() == ch->GetPlayerID())
	{
		if (ch->GetDungeon())
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;673]");
		}
		else
		{
			// ���漺���� ��Ƽ���� ���� �ۿ��� ��Ƽ �ػ� ���ϰ� ����
			if(pParty->IsPartyInDungeon(351))
			{
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;531]");
				return;
			}

			// leader can remove any member
			if (p->pid == ch->GetPlayerID() || pParty->GetMemberCount() == 2)
			{
				// party disband
				CPartyManager::instance().DeleteParty(pParty);
			}
			else
			{
				LPCHARACTER B = CHARACTER_MANAGER::instance().FindByPID(p->pid);
				if (B)
				{
					//pParty->SendPartyRemoveOneToAll(B);
					B->ChatPacket(CHAT_TYPE_INFO, "[LS;675]");
					//pParty->Unlink(B);
					//CPartyManager::instance().SetPartyMember(B->GetPlayerID(), NULL);
				}
				pParty->Quit(p->pid);
			}
		}
	}
	else
	{
		// otherwise, only remove itself
		if (p->pid == ch->GetPlayerID())
		{
			if (ch->GetDungeon())
			{
				ch->ChatPacket(CHAT_TYPE_INFO, "[LS;676]");
			}
			else
			{
				if (pParty->GetMemberCount() == 2)
				{
					// party disband
					CPartyManager::instance().DeleteParty(pParty);
				}
				else
				{
					ch->ChatPacket(CHAT_TYPE_INFO, "[LS;532]");
					//pParty->SendPartyRemoveOneToAll(ch);
					pParty->Quit(ch->GetPlayerID());
					//pParty->SendPartyRemoveAllToOne(ch);
					//CPartyManager::instance().SetPartyMember(ch->GetPlayerID(), NULL);
				}
			}
		}
		else
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;677]");
		}
	}
}

void CInputMain::AnswerMakeGuild(LPCHARACTER ch, const char* c_pData)
{
	TPacketCGAnswerMakeGuild* p = (TPacketCGAnswerMakeGuild*) c_pData;

	if (ch->GetGold() < 200000)
		return;

	if (get_global_time() - ch->GetQuestFlag("guild_manage.new_disband_time") <
			CGuildManager::instance().GetDisbandDelay())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;678;%d]",
				quest::CQuestManager::instance().GetEventFlag("guild_disband_delay"));
		return;
	}

	if (get_global_time() - ch->GetQuestFlag("guild_manage.new_withdraw_time") <
			CGuildManager::instance().GetWithdrawDelay())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;679;%d]",
				quest::CQuestManager::instance().GetEventFlag("guild_withdraw_delay"));
		return;
	}

	if (ch->GetGuild())
		return;

	CGuildManager& gm = CGuildManager::instance();

	TGuildCreateParameter cp;
	memset(&cp, 0, sizeof(cp));

	cp.master = ch;
	strlcpy(cp.name, p->guild_name, sizeof(cp.name));

	if (cp.name[0] == 0 || !check_name(cp.name))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;680]");
		return;
	}

	DWORD dwGuildID = gm.CreateGuild(cp);

	if (dwGuildID)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;682;%s]", cp.name);

		int GuildCreateFee = 200000;

		ch->PointChange(POINT_GOLD, -GuildCreateFee);
		DBManager::instance().SendMoneyLog(MONEY_LOG_GUILD, ch->GetPlayerID(), -GuildCreateFee);

		char Log[128];
		snprintf(Log, sizeof(Log), "GUILD_NAME %s MASTER %s", cp.name, ch->GetName());
		LogManager::instance().CharLog(ch, 0, "MAKE_GUILD", Log);

		ch->RemoveSpecifyItem(GUILD_CREATE_ITEM_VNUM, 1);
		//ch->SendGuildName(dwGuildID);
	}
	else
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;683]");
}

void CInputMain::PartyUseSkill(LPCHARACTER ch, const char* c_pData)
{
	TPacketCGPartyUseSkill* p = (TPacketCGPartyUseSkill*) c_pData;
	if (!ch->GetParty())
		return;

	if (ch->GetPlayerID() != ch->GetParty()->GetLeaderPID())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;684]");
		return;
	}

	switch (p->bySkillIndex)
	{
		case PARTY_SKILL_HEAL:
			ch->GetParty()->HealParty();
			break;
		case PARTY_SKILL_WARP:
			{
				LPCHARACTER pch = CHARACTER_MANAGER::instance().Find(p->vid);
				if (pch)
					ch->GetParty()->SummonToLeader(pch->GetPlayerID());
				else
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Party> Cannot find target to summon."));
			}
			break;
	}
}

void CInputMain::PartyParameter(LPCHARACTER ch, const char * c_pData)
{
	TPacketCGPartyParameter * p = (TPacketCGPartyParameter *) c_pData;
#ifdef __FIX_EXP_GRUP__
	if (ch->GetParty() && ch->GetParty()->GetLeaderPID() == ch->GetPlayerID())
#else
	if (ch->GetParty())
#endif
		ch->GetParty()->SetParameter(p->bDistributeMode);
}
size_t GetSubPacketSize(const GUILD_SUBHEADER_CG& header)
{
	switch (header)
	{
		case GUILD_SUBHEADER_CG_DEPOSIT_MONEY:				return sizeof(int);
		case GUILD_SUBHEADER_CG_WITHDRAW_MONEY:				return sizeof(int);
		case GUILD_SUBHEADER_CG_ADD_MEMBER:					return sizeof(DWORD);
		case GUILD_SUBHEADER_CG_REMOVE_MEMBER:				return sizeof(DWORD);
		case GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME:			return 10;
		case GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY:		return sizeof(BYTE) + sizeof(BYTE);
		case GUILD_SUBHEADER_CG_OFFER:						return sizeof(DWORD);
		case GUILD_SUBHEADER_CG_CHARGE_GSP:					return sizeof(int);
		case GUILD_SUBHEADER_CG_POST_COMMENT:				return 1;
		case GUILD_SUBHEADER_CG_DELETE_COMMENT:				return sizeof(DWORD);
		case GUILD_SUBHEADER_CG_REFRESH_COMMENT:			return 0;
		case GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE:		return sizeof(DWORD) + sizeof(BYTE);
		case GUILD_SUBHEADER_CG_USE_SKILL:					return sizeof(TPacketCGGuildUseSkill);
		case GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL:		return sizeof(DWORD) + sizeof(BYTE);
		case GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER:		return sizeof(DWORD) + sizeof(BYTE);
	}

	return 0;
}

int CInputMain::Guild(LPCHARACTER ch, const char * data, size_t uiBytes)
{
	if (uiBytes < sizeof(TPacketCGGuild))
		return -1;

	const TPacketCGGuild* p = reinterpret_cast<const TPacketCGGuild*>(data);
	const char* c_pData = data + sizeof(TPacketCGGuild);

	uiBytes -= sizeof(TPacketCGGuild);

	const GUILD_SUBHEADER_CG SubHeader = static_cast<GUILD_SUBHEADER_CG>(p->subheader);
	const size_t SubPacketLen = GetSubPacketSize(SubHeader);

	if (uiBytes < SubPacketLen)
	{
		return -1;
	}

	CGuild* pGuild = ch->GetGuild();

	if (NULL == pGuild)
	{
		if (SubHeader != GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, "[LS;523]");
			return SubPacketLen;
		}
	}

	switch (SubHeader)
	{
		case GUILD_SUBHEADER_CG_DEPOSIT_MONEY:
			{
				// by mhh : 길드자금은 당분간 넣을 수 없다.
				return SubPacketLen;

#ifdef ENABLE_REMOVE_LIMIT_GOLD
				const int gold = std::min<unsigned long long>(*reinterpret_cast<const int*>(c_pData), __deposit_limit());
#else
				const int gold = MIN(*reinterpret_cast<const int*>(c_pData), __deposit_limit());
#endif

				if (gold < 0)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Wrong amount."));
					return SubPacketLen;
				}

#ifdef ENABLE_REMOVE_LIMIT_GOLD
				if (ch->GetGold() < static_cast<unsigned long long>(gold))
#else
				if (ch->GetGold() < gold)
#endif
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> You don't have enough money."));
					return SubPacketLen;
				}

				pGuild->RequestDepositMoney(ch, gold);
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_WITHDRAW_MONEY:
			{
				// by mhh : 길드자금은 당분간 뺄 수 없다.
				return SubPacketLen;

#ifdef ENABLE_REMOVE_LIMIT_GOLD
				const  long gold = std::min<long>(*reinterpret_cast<const long*>(c_pData), 500000);
#else
				const int gold = MIN(*reinterpret_cast<const int*>(c_pData), 500000);
#endif

				if (gold < 0)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Wrong amount."));
					return SubPacketLen;
				}

				pGuild->RequestWithdrawMoney(ch, gold);
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_ADD_MEMBER:
			{
				const DWORD vid = *reinterpret_cast<const DWORD*>(c_pData);
				LPCHARACTER newmember = CHARACTER_MANAGER::instance().Find(vid);

				if (!newmember)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Couldn't find such a person."));
					return SubPacketLen;
				}

				// @fixme145 BEGIN (+newmember ispc check)
				if (!ch->IsPC() || !newmember->IsPC())
					return SubPacketLen;
				// @fixme145 END

				pGuild->Invite(ch, newmember);
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_REMOVE_MEMBER:
			{
				if (pGuild->UnderAnyWar() != 0)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> You cannot leave a guild member during a Guild War."));
					return SubPacketLen;
				}

				const DWORD pid = *reinterpret_cast<const DWORD*>(c_pData);
				const TGuildMember* m = pGuild->GetMember(ch->GetPlayerID());

				if (NULL == m)
					return -1;

				LPCHARACTER member = CHARACTER_MANAGER::instance().FindByPID(pid);

				if (member)
				{
					if (member->GetGuild() != pGuild)
					{
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> The opponent is not from the same guild."));
						return SubPacketLen;
					}

					if (!pGuild->HasGradeAuth(m->grade, GUILD_AUTH_REMOVE_MEMBER))
					{
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> You do not have the right to forcibly leave the guild."));
						return SubPacketLen;
					}

					member->SetQuestFlag("guild_manage.new_withdraw_time", get_global_time());
					pGuild->RequestRemoveMember(member->GetPlayerID());

					if (g_bGuildInviteLimit)
					{
						DBManager::instance().Query("REPLACE INTO guild_invite_limit VALUES(%d, %d)", pGuild->GetID(), get_global_time());
					}
				}
				else
				{
					if (!pGuild->HasGradeAuth(m->grade, GUILD_AUTH_REMOVE_MEMBER))
					{
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> You do not have the right to forcibly leave the guild."));
						return SubPacketLen;
					}

					if (pGuild->RequestRemoveMember(pid))
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Guild members were forced to leave."));
					else
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Couldn't find such a person."));
				}
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_CHANGE_GRADE_NAME:
			{
				char gradename[GUILD_GRADE_NAME_MAX_LEN + 1];
				strlcpy(gradename, c_pData + 1, sizeof(gradename));

				const TGuildMember * m = pGuild->GetMember(ch->GetPlayerID());

				if (NULL == m)
					return -1;

				if (m->grade != GUILD_LEADER_GRADE)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> You do not have permission to change the position name."));
				}
				else if (*c_pData == GUILD_LEADER_GRADE)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Guild leader position names cannot be changed."));
				}
				else if (!check_name(gradename))
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Invalid position name."));
				}
				else
				{
					pGuild->ChangeGradeName(*c_pData, gradename);
				}
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_CHANGE_GRADE_AUTHORITY:
			{
				const TGuildMember* m = pGuild->GetMember(ch->GetPlayerID());

				if (NULL == m)
					return -1;

				if (m->grade != GUILD_LEADER_GRADE)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> You do not have the right to change the position authority."));
				}
				else if (*c_pData == GUILD_LEADER_GRADE)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> The guild leader's authority cannot be changed."));
				}
				else
				{
					pGuild->ChangeGradeAuth(*c_pData, *(c_pData + 1));
				}
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_OFFER:
			{
				DWORD offer = *reinterpret_cast<const DWORD*>(c_pData);

				if (pGuild->GetLevel() >= GUILD_MAX_LEVEL)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Your guild is already at the highest level."));
				}
				else
				{
					offer /= 100;
					offer *= 100;

					if (pGuild->OfferExp(ch, offer))
					{
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Invested %u experience."), offer);
					}
					else
					{
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Failed to invest in EXP."));
					}
				}
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_CHARGE_GSP:
			{
				const int offer = *reinterpret_cast<const int*>(c_pData);
				const int gold = offer * 100;

				if (offer < 0 || gold < offer || gold < 0 || ch->GetGold() < gold)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Money is insufficient."));
					return SubPacketLen;
				}

				if (!pGuild->ChargeSP(ch, offer))
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> Failed to recover Dragon Faith."));
				}
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_POST_COMMENT:
			{
				const size_t length = *c_pData;

				if (length > GUILD_COMMENT_MAX_LEN)
				{
					// �߸��� ����.. ��������.
					sys_err("POST_COMMENT: %s comment too long (length: %u)", ch->GetName(), length);
					ch->GetDesc()->SetPhase(PHASE_CLOSE);
					return -1;
				}

				if (uiBytes < 1 + length)
					return -1;

				const TGuildMember* m = pGuild->GetMember(ch->GetPlayerID());

				if (NULL == m)
					return -1;

				if (length && !pGuild->HasGradeAuth(m->grade, GUILD_AUTH_NOTICE) && *(c_pData + 1) == '!')
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> You do not have permission to write notices."));
				}
				else
				{
					std::string str(c_pData + 1, length);
					pGuild->AddComment(ch, str);
				}

				return (1 + length);
			}

		case GUILD_SUBHEADER_CG_DELETE_COMMENT:
			{
				const DWORD comment_id = *reinterpret_cast<const DWORD*>(c_pData);

				pGuild->DeleteComment(ch, comment_id);
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_REFRESH_COMMENT:
			pGuild->RefreshComment(ch);
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_CHANGE_MEMBER_GRADE:
			{
				const DWORD pid = *reinterpret_cast<const DWORD*>(c_pData);
				const BYTE grade = *(c_pData + sizeof(DWORD));
				const TGuildMember* m = pGuild->GetMember(ch->GetPlayerID());

				if (NULL == m)
					return -1;

				if (m->grade != GUILD_LEADER_GRADE)
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> You do not have the right to change positions."));
				else if (ch->GetPlayerID() == pid)
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> The position of guild leader cannot be changed."));
				else if (grade == 1)
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> You cannot change your position to guild leader."));
				else
					pGuild->ChangeMemberGrade(pid, grade);
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_USE_SKILL:
			{
				const TPacketCGGuildUseSkill* p = reinterpret_cast<const TPacketCGGuildUseSkill*>(c_pData);

				pGuild->UseSkill(p->dwVnum, ch, p->dwPID);
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_CHANGE_MEMBER_GENERAL:
			{
				const DWORD pid = *reinterpret_cast<const DWORD*>(c_pData);
				const BYTE is_general = *(c_pData + sizeof(DWORD));
				const TGuildMember* m = pGuild->GetMember(ch->GetPlayerID());

				if (NULL == m)
					return -1;

				if (m->grade != GUILD_LEADER_GRADE)
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> You do not have the right to appoint a general."));
				}
				else
				{
					if (!pGuild->ChangeMemberGeneral(pid, is_general))
					{
						ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("<Guild> You can no longer specify a longevity."));
					}
				}
			}
			return SubPacketLen;

		case GUILD_SUBHEADER_CG_GUILD_INVITE_ANSWER:
			{
				const DWORD guild_id = *reinterpret_cast<const DWORD*>(c_pData);
				const BYTE accept = *(c_pData + sizeof(DWORD));

				CGuild * g = CGuildManager::instance().FindGuild(guild_id);

				if (g)
				{
					if (accept)
						g->InviteAccept(ch);
					else
						g->InviteDeny(ch->GetPlayerID());
				}
			}
			return SubPacketLen;

	}

	return 0;
}

void CInputMain::Fishing(LPCHARACTER ch, const char* c_pData)
{
	TPacketCGFishing* p = (TPacketCGFishing*)c_pData;
	ch->SetRotation(p->dir * 5);
	ch->fishing();
	return;
}

void CInputMain::ItemGive(LPCHARACTER ch, const char* c_pData)
{
	TPacketCGGiveItem* p = (TPacketCGGiveItem*) c_pData;
	LPCHARACTER to_ch = CHARACTER_MANAGER::instance().Find(p->dwTargetVID);

	if (to_ch)
		ch->GiveItem(to_ch, p->ItemPos);
	else
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;710]");
}



#ifdef ENABLE_GAYA_SYSTEM
void CInputMain::GayaSystemSend(LPCHARACTER ch, const char * data)
{
	struct packet_gaya_system * pinfo = (struct packet_gaya_system *) data;
	switch (pinfo->subheader)
	{
		case GAYA_SYSTEM_SUB_HEADER_CRAFT:
		{
			int pos = pinfo->pos;
			ch->CraftGayaItems(pos);

		}
		break;

		case GAYA_SYSTEM_SUB_HEADER_MARKET:
		{
			int pos = pinfo->pos;
			ch->MarketGayaItems(pos);

		}
		break;

		case GAYA_SYSTEM_SUB_HEADER_REFRESH:
		{
			ch->RefreshGayaItems();
		}
		break;
	}
}
#endif


void CInputMain::Hack(LPCHARACTER ch, const char * c_pData)
{
	TPacketCGHack * p = (TPacketCGHack *) c_pData;

	char buf[sizeof(p->szBuf)];
	strlcpy(buf, p->szBuf, sizeof(buf));

	sys_err("HACK_DETECT: %s %s", ch->GetName(), buf);

	// ���� Ŭ���̾�Ʈ���� �� ��Ŷ�� ������ ��찡 �����Ƿ� ������ ������ �Ѵ�
	ch->GetDesc()->SetPhase(PHASE_CLOSE);
}

#ifdef ENABLE_REFINE_ELEMENT
void CInputMain::RefineElement(LPCHARACTER ch, const char* c_pData)
{
	const TPacketCGRefineElement* p = reinterpret_cast<const TPacketCGRefineElement*>(c_pData);

	if (ch->GetExchange() || ch->IsOpenSafebox() || ch->IsCubeOpen())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Can't do this right now"));
		ch->ClearRefineElement();
		return;
	}

	// Window close
	if (p->bArg == 255)
	{
		ch->ClearRefineElement();
		return;
	}
	
	ch->DoRefineElement(p->bArg);
}
#endif

int CInputMain::MyShop(LPCHARACTER ch, const char * c_pData, size_t uiBytes)
{	
	TPacketCGMyShop * p = (TPacketCGMyShop *) c_pData;
	int iExtraLen = p->bCount * sizeof(TShopItemTable);
	
#ifdef SHOP_BLOCK_GAME99
	if (g_bChannel >= 99 & ch->GetGMLevel() <= 0) {
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT( "SHOP_OX_ERROR"));
		return (iExtraLen);
	}
#endif

#ifdef OFFLINE_SHOP
	if (quest::CQuestManager::instance().GetEventFlag("shop_off") == 1)
		if (ch)
		{
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("SHOP_TEMP_OFF"));
			return (iExtraLen);
		}
#endif

#ifdef SHOP_GM_PRIVILEGES
	if (GM_PLAYER < ch->GetGMLevel() && SHOP_GM_PRIVILEGES >ch->GetGMLevel()) {
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("SHOP_PRIV_ERROR"));
		return (iExtraLen);
	}
#endif

	if (uiBytes < sizeof(TPacketCGMyShop) + iExtraLen)
		return -1;
    
#ifdef ENABLE_REMOVE_LIMIT_GOLD
	if (ch->GetGold() >= GOLD_MAX_MAX)
#else
	if (ch->GetGold() >= GOLD_MAX)
#endif
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You reached the yang limit."));
		sys_log(0, "MyShop ==> OverFlow Gold id %u name %s ", ch->GetPlayerID(), ch->GetName());
		return (iExtraLen);
	}


	if (ch->IsStun() || ch->IsDead())
		return (iExtraLen);
	
	if (ch->GetExchange() || ch->IsOpenSafebox() || ch->GetShopOwner() || ch->IsCubeOpen() || ch->IsAuraRefineWindowOpen())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1014]");
		return (iExtraLen);
	}

	sys_log(0, "MyShop count %d", p->bCount);
#ifdef OFFLINE_SHOP
	ch->OpenMyShop(p->szSign, (TShopItemTable *)(c_pData + sizeof(TPacketCGMyShop)), p->bCount, p->days);
#else
	ch->OpenMyShop(p->szSign, (TShopItemTable *)(c_pData + sizeof(TPacketCGMyShop)), p->bCount);
#endif
	return (iExtraLen);
}

void CInputMain::Refine(LPCHARACTER ch, const char* c_pData)
{
	const TPacketCGRefine* p = reinterpret_cast<const TPacketCGRefine*>(c_pData);

	if (ch->GetExchange() || ch->IsOpenSafebox() || ch->GetShopOwner() || ch->GetMyShop() || ch->IsCubeOpen() || ch->IsAuraRefineWindowOpen())
	{
		ch->ChatPacket(CHAT_TYPE_INFO,  "[LS;1014]");
		ch->ClearRefineMode();
		return;
	}

	if (p->type == 255)
	{
		// DoRefine Cancel
		ch->ClearRefineMode();
		return;
	}

#ifdef ENABLE_CUSTOM_INVENTORY
	if (p->pos >= ch->Inventory_Size() && (p->pos < CUSTOM_INVENTORY_SLOT_START || p->pos >= CUSTOM_INVENTORY_SLOT_END))
#else
	if (p->pos >= INVENTORY_MAX_NUM)
#endif
	{
		ch->ClearRefineMode();
		return;
	}

	LPITEM item = ch->GetInventoryItem(p->pos);

	if (!item)
	{
		ch->ClearRefineMode();
		return;
	}

	ch->SetRefineTime();

	if (p->type == REFINE_TYPE_NORMAL)
	{
		sys_log (0, "refine_type_noraml");
		ch->DoRefine(item);
	}
	else if (p->type == REFINE_TYPE_SCROLL || p->type == REFINE_TYPE_HYUNIRON || p->type == REFINE_TYPE_MUSIN || p->type == REFINE_TYPE_BDRAGON)
	{
		sys_log (0, "refine_type_scroll, ...");
		ch->DoRefineWithScroll(item);
	}
	else if (p->type == REFINE_TYPE_MONEY_ONLY)
	{
		const LPITEM item = ch->GetInventoryItem(p->pos);

		if (NULL != item)
		{
			if (500 <= item->GetRefineSet())
			{
				LogManager::instance().HackLog("DEVIL_TOWER_REFINE_HACK", ch);
			}
			else
			{
				if (ch->GetQuestFlag("deviltower_zone.can_refine"))
				{
					ch->DoRefine(item, true);
					ch->SetQuestFlag("deviltower_zone.can_refine", 0);
				}
				else
				{
					ch->ChatPacket(CHAT_TYPE_INFO, "[LS;1067]");
				}
			}
		}
	}

	ch->ClearRefineMode();
}

#ifdef __SASH_SYSTEM__
void CInputMain::Sash(LPCHARACTER pkChar, const char* c_pData)
{
	quest::PC * pPC = quest::CQuestManager::instance().GetPCForce(pkChar->GetPlayerID());
	if (pPC->IsRunning())
		return;
	
	TPacketSash * sPacket = (TPacketSash*) c_pData;
	switch (sPacket->subheader)
	{
		case SASH_SUBHEADER_CG_CLOSE:
			{
				pkChar->CloseSash();
			}
			break;
		case SASH_SUBHEADER_CG_ADD:
			{
				pkChar->AddSashMaterial(sPacket->tPos, sPacket->bPos);
			}
			break;
		case SASH_SUBHEADER_CG_REMOVE:
			{
				pkChar->RemoveSashMaterial(sPacket->bPos);
			}
			break;
		case SASH_SUBHEADER_CG_REFINE:
			{
				pkChar->RefineSashMaterials();
			}
			break;
		default:
			break;
	}
}
#endif

#ifdef __CHANGELOOK_SYSTEM__
void CInputMain::ChangeLook(LPCHARACTER pkChar, const char* c_pData)
{
	quest::PC * pPC = quest::CQuestManager::instance().GetPCForce(pkChar->GetPlayerID());
	if (pPC->IsRunning())
		return;
	
	TPacketChangeLook * sPacket = (TPacketChangeLook*) c_pData;
	switch (sPacket->subheader)
	{
		case CL_SUBHEADER_CLOSE:
			{
				pkChar->ChangeLookWindow(false);
			}
			break;
		case CL_SUBHEADER_ADD:
			{
				pkChar->AddClMaterial(sPacket->tPos, sPacket->bPos);
			}
			break;
		case CL_SUBHEADER_REMOVE:
			{
				pkChar->RemoveClMaterial(sPacket->bPos);
			}
			break;
		case CL_SUBHEADER_REFINE:
			{
				pkChar->RefineClMaterials();
			}
			break;
		default:
			break;
	}
}
#endif

#ifdef ENABLE_CUBE_RENEWAL_WORLDARD
void CInputMain::CubeRenewalSend(LPCHARACTER ch, const char* data)
{
	struct packet_send_cube_renewal * pinfo = (struct packet_send_cube_renewal *) data;
	switch (pinfo->subheader)
	{
		case CUBE_RENEWAL_SUB_HEADER_MAKE_ITEM:
		{

			int index_item = pinfo->index_item;
			int count_item = pinfo->count_item;
			int index_item_improve = pinfo->index_item_improve;

			Cube_Make(ch,index_item,count_item,index_item_improve);
		}
		break;

		case CUBE_RENEWAL_SUB_HEADER_CLOSE:
		{
			Cube_close(ch);
		}
		break;
	}
}
#endif
#if defined(__ATTR_6TH_7TH__)
void CInputMain::Attr67Add(LPCHARACTER ch, const char* c_pData)
{
	const TPacketCGAttr67Add* pkPacket = (TPacketCGAttr67Add*)c_pData;
	switch (pkPacket->bySubHeader)
	{
	case SUBHEADER_CG_ATTR67_ADD_CLOSE:
		ch->SetOpenAttr67Add(false);
		break;
	case SUBHEADER_CG_ATTR67_ADD_OPEN:
		if (!ch->IsOpenAttr67Add())
			ch->SetOpenAttr67Add(true);
		break;
	case SUBHEADER_CG_ATTR67_ADD_REGIST:
		if (ch->IsOpenAttr67Add())
			ch->Attr67Add(pkPacket->Attr67AddData);
		break;
	default:
		return;
	}
}
#endif
#if defined(__DAILY_GIFT_SYSTEM__)
void CInputMain::DailyGift(LPCHARACTER ch, const char* c_pData)
{
	TPacketCGDailyGift* packetCG = (TPacketCGDailyGift*)c_pData;

	switch (packetCG->bAction)
	{
	case DAILY_GIFT_CLOSE:
	{
		if (ch->CheckDailyGiftStatus())
			ch->CloseDailyGift();
	}
	break;

	case DAILY_GIFT_OPEN:
	{
		if (ch->CheckDailyGiftStatus())
			ch->LoadDailyGiftWeek();
	}
	break;

	case DAILY_GIFT_COLLECT:
	{
		if (ch->CheckDailyGiftStatus())
			ch->CollectDailyGift(packetCG->bSlotIndex, /*bUseItem*/ false);
	}
	break;

	case DAILY_GIFT_COLLECT_USE_TICKET:
	{
		if (ch->CheckDailyGiftStatus())
			ch->CollectDailyGift(packetCG->bSlotIndex, /*bUseItem*/ true);
	}
	break;

	}
}
#endif

#if defined(__WORLD_BOSS_EVENT__)
#include "worldboss.h"
void CInputMain::WorldBoss(const LPCHARACTER c_lpChar, const char* c_pszData)
{
	const TPacketCGWorldBoss* c_pPacket = reinterpret_cast<const TPacketCGWorldBoss*>(c_pszData);
	switch (c_pPacket->bSubHeader)
	{
	case WORLD_BOSS_SUBHEADER_CG_INFO:
		CWorldBoss::instance().Process(c_lpChar, c_pPacket->bSubHeader);
		break;

	case WORLD_BOSS_SUBHEADER_CG_REWARD:
		CWorldBoss::instance().Reward(c_lpChar);
		break;

	case WORLD_BOSS_SUBHEADER_CG_RANKING:
		CWorldBoss::instance().GetRanking(c_lpChar);
		break;
	}

}
#endif


#ifdef __MULTI_LANGUAGE_SYSTEM__
void CInputMain::ChangeLanguage(LPCHARACTER ch, BYTE bLanguage)
{
	if (!ch)
		return;

	if (!ch->GetDesc())
		return;

	BYTE bCurrentLanguage = ch->GetDesc()->GetLanguage();

	if (bCurrentLanguage == bLanguage)
		return;

	if (bLanguage > LOCALE_YMIR && bLanguage < LOCALE_MAX_NUM)
	{
		TRequestChangeLanguage packet;
		packet.dwAID = ch->GetDesc()->GetAccountTable().id;
		packet.bLanguage = bLanguage;

		db_clientdesc->DBPacketHeader(HEADER_GD_REQUEST_CHANGE_LANGUAGE, 0, sizeof(TRequestChangeLanguage));
		db_clientdesc->Packet(&packet, sizeof(packet));

		ch->ChangeLanguage(bLanguage);
	}
}
#endif

#ifdef __EXTENDED_WHISPER_DETAILS__
void CInputMain::WhisperDetails(LPCHARACTER ch, const char* c_pData)
{
	TPacketCGWhisperDetails* CGWhisperDetails = (TPacketCGWhisperDetails*)c_pData;

	if (!*CGWhisperDetails->name)
		return;

	TPacketGCWhisperDetails GCWhisperDetails;
	GCWhisperDetails.header = HEADER_GC_WHISPER_DETAILS;
	strncpy(GCWhisperDetails.name, CGWhisperDetails->name, sizeof(GCWhisperDetails.name) - 1);

	BYTE bLanguage = LOCALE_DEFAULT;

	LPCHARACTER pkChr = CHARACTER_MANAGER::instance().FindPC(CGWhisperDetails->name);

	if (!pkChr)
	{
		LPDESC pkDesc = NULL;
		CCI* pkCCI = P2P_MANAGER::instance().Find(CGWhisperDetails->name);

		if (pkCCI)
		{
			pkDesc = pkCCI->pkDesc;
			if (pkDesc)
				bLanguage = pkCCI->bLanguage;
		}
	}
	else
	{
		if (pkChr->GetDesc())
			bLanguage = pkChr->GetDesc()->GetLanguage();
	}

	GCWhisperDetails.bLanguage = bLanguage;
	ch->GetDesc()->Packet(&GCWhisperDetails, sizeof(GCWhisperDetails));
}
#endif



int CInputMain::Analyze(LPDESC d, BYTE bHeader, const char * c_pData)
{
	LPCHARACTER ch;

	if (!(ch = d->GetCharacter()))
	{
		sys_err("no character on desc");
		d->SetPhase(PHASE_CLOSE);
		return (0);
	}

	int iExtraLen = 0;

	if (test_server && bHeader != HEADER_CG_MOVE)
		sys_log(0, "CInputMain::Analyze() ==> Header [%d] ", bHeader);

	switch (bHeader)
	{
		case HEADER_CG_PONG:
			Pong(d);
			break;

		case HEADER_CG_TIME_SYNC:
			Handshake(d, c_pData);
			break;

		case HEADER_CG_CHAT:
			if (test_server)
			{
				char* pBuf = (char*)c_pData;
				sys_log(0, "%s", pBuf + sizeof(TPacketCGChat));
			}

			if ((iExtraLen = Chat(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
			break;
		case HEADER_CG_WHISPER:
			if ((iExtraLen = Whisper(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
			break;
		case HEADER_CG_MOVE:
			Move(ch, c_pData);
			break;
		case HEADER_CG_CHARACTER_POSITION:
			Position(ch, c_pData);
			break;
		case HEADER_CG_ITEM_USE:
			if (!ch->IsObserverMode())
				ItemUse(ch, c_pData);
			break;
		case HEADER_CG_ITEM_DROP:
			if (!ch->IsObserverMode())
			{
				ItemDrop(ch, c_pData);
			}
			break;
		case HEADER_CG_ITEM_DROP2:
			if (!ch->IsObserverMode())
				ItemDrop2(ch, c_pData);
			break;
		case HEADER_CG_ITEM_DESTROY:
			if (!ch->IsObserverMode())
				ItemDestroy(ch, c_pData);
		break;
		case HEADER_CG_ITEM_MOVE:
			if (!ch->IsObserverMode())
				ItemMove(ch, c_pData);
			break;
		case ENVANTER_BLACK:
			if (!ch->IsObserverMode())
				InventoryExpansion(ch, c_pData);
		break;
		case HEADER_CG_ITEM_PICKUP:
			if (!ch->IsObserverMode())
				ItemPickup(ch, c_pData);
			break;
		case HEADER_CG_ITEM_USE_TO_ITEM:
			if (!ch->IsObserverMode())
				ItemToItem(ch, c_pData);
			break;
		case HEADER_CG_ITEM_GIVE:
			if (!ch->IsObserverMode())
				ItemGive(ch, c_pData);
			break;
		case HEADER_CG_EXCHANGE:
			if (!ch->IsObserverMode())
				Exchange(ch, c_pData);
			break;
		case HEADER_CG_ATTACK:
		case HEADER_CG_SHOOT:
			if (!ch->IsObserverMode())
			{
				Attack(ch, bHeader, c_pData);
			}
			break;
		case HEADER_CG_USE_SKILL:
			if (!ch->IsObserverMode())
				UseSkill(ch, c_pData);
			break;
		case HEADER_CG_QUICKSLOT_ADD:
			QuickslotAdd(ch, c_pData);
			break;
		case HEADER_CG_QUICKSLOT_DEL:
			QuickslotDelete(ch, c_pData);
			break;
#if defined(__WORLD_BOSS_EVENT__)
		case HEADER_CG_WORLD_BOSS:
			WorldBoss(ch, c_pData);
			break;
#endif
		case HEADER_CG_QUICKSLOT_SWAP:
			QuickslotSwap(ch, c_pData);
			break;
		case HEADER_CG_SHOP:
			if ((iExtraLen = Shop(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
			break;
		case HEADER_CG_FISH_EVENT_SEND:
			if ((iExtraLen = FishEvent(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
			break;

		case HEADER_CG_MESSENGER:
			if ((iExtraLen = Messenger(ch, c_pData, m_iBufferLeft))<0)
				return -1;
			break;
		case HEADER_CG_ON_CLICK:
			OnClick(ch, c_pData);
			break;
		case HEADER_CG_SYNC_POSITION:
			if ((iExtraLen = SyncPosition(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
			break;
		case HEADER_CG_ADD_FLY_TARGETING:
		case HEADER_CG_FLY_TARGETING:
			FlyTarget(ch, c_pData, bHeader);
			break;
		case HEADER_CG_SCRIPT_BUTTON:
			ScriptButton(ch, c_pData);
			break;
		case HEADER_CG_SCRIPT_SELECT_ITEM:
			ScriptSelectItem(ch, c_pData);
			break;
		case HEADER_CG_SCRIPT_ANSWER:
			ScriptAnswer(ch, c_pData);
			break;
		case HEADER_CG_QUEST_INPUT_STRING:
			QuestInputString(ch, c_pData);
			break;
		case HEADER_CG_QUEST_CONFIRM:
			QuestConfirm(ch, c_pData);
			break;
		case HEADER_CG_REQUEST_EVENT_QUEST:
			RequestEventQuest(ch, c_pData);
			break;
		case HEADER_CG_TARGET:
			Target(ch, c_pData);
			break;
		case HEADER_CG_WARP:
			Warp(ch, c_pData);
			break;
		case HEADER_CG_SAFEBOX_CHECKIN:
			SafeboxCheckin(ch, c_pData);
			break;
		case HEADER_CG_SAFEBOX_CHECKOUT:
			SafeboxCheckout(ch, c_pData, false);
			break;
		case HEADER_CG_SAFEBOX_ITEM_MOVE:
			SafeboxItemMove(ch, c_pData);
			break;
		case HEADER_CG_MALL_CHECKOUT:
			SafeboxCheckout(ch, c_pData, true);
			break;
		case HEADER_CG_PARTY_INVITE:
			PartyInvite(ch, c_pData);
			break;
		case HEADER_CG_PARTY_REMOVE:
			PartyRemove(ch, c_pData);
			break;
		case HEADER_CG_PARTY_INVITE_ANSWER:
			PartyInviteAnswer(ch, c_pData);
			break;
		case HEADER_CG_PARTY_SET_STATE:
			PartySetState(ch, c_pData);
			break;
		case HEADER_CG_PARTY_USE_SKILL:
			PartyUseSkill(ch, c_pData);
			break;
		case HEADER_CG_PARTY_PARAMETER:
			PartyParameter(ch, c_pData);
			break;
		case HEADER_CG_ANSWER_MAKE_GUILD:
			AnswerMakeGuild(ch, c_pData);
			break;
		case HEADER_CG_GUILD:
			if ((iExtraLen = Guild(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
			break;
		case HEADER_CG_FISHING:
			Fishing(ch, c_pData);
			break;
		case HEADER_CG_HACK:
			Hack(ch, c_pData);
			break;
		case HEADER_CG_MYSHOP:
			if ((iExtraLen = MyShop(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
			break;
		case HEADER_CG_REFINE:
			Refine(ch, c_pData);
			break;
		case HEADER_CG_REFINE_ELEMENT:
			RefineElement(ch, c_pData);
			break;
		case HEADER_CG_CL:
			{
				ChangeLook(ch, c_pData);
			}
			break;
		case HEADER_CG_SASH:
			Sash(ch, c_pData);
			break;
		case HEADER_CG_AURA:
			if ((iExtraLen = Aura(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
			break;
	case HEADER_CG_INVENTORY_PROTECTED:
		RecvActivateProtectedSystem(ch, c_pData);
		break;
		case HEADER_CG_CLIENT_VERSION:
			Version(ch, c_pData);
			break;

		case HEADER_CG_TARGET_INFO_LOAD:
			{
				TargetInfoLoad(ch, c_pData);
			}
			break;
		case HEADER_CG_GAYA_SYSTEM:
			GayaSystemSend(ch, c_pData);
			break;

		case HEADER_CG_DRAGON_SOUL_REFINE:
			{
				TPacketCGDragonSoulRefine* p = reinterpret_cast <TPacketCGDragonSoulRefine*>((void*)c_pData);
				switch(p->bSubType)
				{
                case DS_SUB_HEADER_OPEN:
                    ch->DragonSoul_RefineWindow_Open(ch);
                    break;
					
				case DS_SUB_HEADER_CLOSE:
					ch->DragonSoul_RefineWindow_Close();
					break;
				case DS_SUB_HEADER_DO_REFINE_GRADE:
					{
						DSManager::instance().DoRefineGrade(ch, p->ItemGrid);
					}
					break;
				case DS_SUB_HEADER_DO_REFINE_STEP:
					{
						DSManager::instance().DoRefineStep(ch, p->ItemGrid);
					}
					break;
				case DS_SUB_HEADER_DO_REFINE_STRENGTH:
					{
						DSManager::instance().DoRefineStrength(ch, p->ItemGrid);
					}
					break;
                case DS_SUB_HEADER_REFINE_ALL: 
					{
						DSManager::instance().DoAllRefineGrade(ch, p->ItemGrid[0].cell, p->ItemGrid[1].cell);
					} 
					break;	
				}
			}
			break;
		case HEADER_CG_REQUEST_EVENT_DATA:
			RequestEventData(ch, c_pData);
			break;
		case HEADER_CG_SWITCHBOT:
			if ((iExtraLen = Switchbot(ch, c_pData, m_iBufferLeft)) < 0)
			{
				return -1;
			}
			break;
		case HEADER_CG_CUBE_RENEWAL:
			CubeRenewalSend(ch, c_pData);
			break;
		case HEADER_CG_CHANGE_LANGUAGE:
		{
			TPacketChangeLanguage* p = reinterpret_cast <TPacketChangeLanguage*>((void*)c_pData);
			ChangeLanguage(ch, p->bLanguage);
		}
			break;
		case HEADER_CG_WHISPER_DETAILS:
			WhisperDetails(ch, c_pData);
			break;

		case HEADER_CG_ATTR67_ADD:
			Attr67Add(ch, c_pData);
			break;
		case HEADER_CG_DAILY_GIFT:
			DailyGift(ch, c_pData);
			break;
	case HEADER_CG_PREMIUM_PLAYERS:
		RecvPremiumPlayersPacket(ch, c_pData);
		break;
	case HEADER_CG_BIOLOGIST:
		RecvBiologistPacket(ch, c_pData);
		break;
	case HEADER_CG_PRIVATE_SHOP:
	{
		TPacketCGPrivateShop* p = (TPacketCGPrivateShop*)c_pData;
		c_pData += sizeof(TPacketCGPrivateShop);

		switch (p->bSubHeader)
		{
		case SUBHEADER_CG_PRIVATE_SHOP_BUILD:
		{
			if ((iExtraLen = PrivateShopBuild(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_CLOSE:
		{
			PrivateShopClose(ch);
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_PANEL_OPEN:
		{
			PrivateShopPanelOpen(ch);
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_PANEL_CLOSE:
		{
			PrivateShopPanelClose(ch);
		} break;
			
			case SUBHEADER_CG_PRIVATE_SHOP_START:
		{
			if ((iExtraLen = PrivateShopStart(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_END:
		{
			PrivateShopEnd(ch);
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_BUY:
		{
			if ((iExtraLen = PrivateShopBuy(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_WITHDRAW:
		{
			PrivateShopWithdraw(ch);
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_MODIFY:
		{
			PrivateShopModify(ch);
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_ITEM_PRICE_CHANGE:
		{
			if ((iExtraLen = PrivateShopItemPriceChange(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_ITEM_MOVE:
		{
			if ((iExtraLen = PrivateShopItemMove(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_ITEM_CHECKIN:
		{
			if ((iExtraLen = PrivateShopItemCheckin(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_ITEM_CHECKOUT:
		{
			if ((iExtraLen = PrivateShopItemCheckout(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_TITLE_CHANGE:
		{
			if ((iExtraLen = PrivateShopTitleChange(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_WARP_REQUEST:
		{
			PrivateShopWarpRequest(ch);
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_SLOT_UNLOCK_REQUEST:
		{
			if ((iExtraLen = PrivateShopSlotUnlockRequest(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_SEARCH_CLOSE:
		{
			PrivateShopSearchClose(ch);
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_SEARCH:
		{
			if ((iExtraLen = PrivateShopSearch(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_SEARCH_BUY:
		{
			if ((iExtraLen = PrivateShopSearchBuy(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_MARKET_ITEM_PRICE_DATA_REQUEST:
		{
			PrivateShopMarketItemPriceDataRequest(ch);
		} break;

		case SUBHEADER_CG_PRIVATE_SHOP_MARKET_ITEM_PRICE_REQUEST:
		{
			if ((iExtraLen = PrivateShopMarketItemPriceRequest(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;
		} break;

			default:
				sys_err("Unknown private shop subheader %u", p->bSubHeader);
				break;
			}
		} break;

	}
	return (iExtraLen);
}

int CInputDead::Analyze(LPDESC d, BYTE bHeader, const char * c_pData)
{
	LPCHARACTER ch;

	if (!(ch = d->GetCharacter()))
	{
		sys_err("no character on desc");
		return 0;
	}

	int iExtraLen = 0;

	switch (bHeader)
	{
		case HEADER_CG_PONG:
			Pong(d);
			break;

		case HEADER_CG_TIME_SYNC:
			Handshake(d, c_pData);
			break;

		case HEADER_CG_CHAT:
			if ((iExtraLen = Chat(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;

			break;

		case HEADER_CG_WHISPER:
			if ((iExtraLen = Whisper(ch, c_pData, m_iBufferLeft)) < 0)
				return -1;

			break;

		case HEADER_CG_HACK:
			Hack(ch, c_pData);
			break;

		default:
			return (0);
	}

	return (iExtraLen);
}



#ifdef __PREMIUM_PRIVATE_SHOP__
int CInputMain::PrivateShopBuild(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	TPacketCGPrivateShopBuild* p = (TPacketCGPrivateShopBuild*)c_pData;
	c_pData += sizeof(TPacketCGPrivateShopBuild);

	// Size of the remaining packet
	int iExtraLen = sizeof(TPacketCGPrivateShopBuild) + p->wItemCount * sizeof(TPrivateShopItem);

	// No items have been passed
	if (uiBytes < (sizeof(TPacketCGPrivateShop) + iExtraLen))
		return -1;

	if (!ch || !ch->GetDesc())
		return iExtraLen;

	if (ch && ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Your logout has been canceled."));
		event_cancel(&ch->m_pkTimedEvent);

		return iExtraLen;
	}

	if (ch->IsStun() || ch->IsDead())
		return iExtraLen;


#ifdef ENABLE_PRIVATE_SHOP_BUILD_LIMITATIONS
	if (!CanBuildPrivateShop(ch))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot open a personal shop while another window is open."));
		return iExtraLen;
	}
#endif

	if (thecore_pulse() - ch->GetLastPrivateShopBuildTime() < PASSES_PER_SEC(10))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a few moments before building your personal shop again."));
		return iExtraLen;
	}

	if (!CheckTradeWindows(ch))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot open a personal shop while another window is open."));
		return iExtraLen;
	}

	ch->BuildPrivateShop(p->szTitle, p->dwPolyVnum, p->bTitleType, p->bPageCount, p->wItemCount, (TPrivateShopItem*)c_pData);
	ch->SetLastPrivateShopBuildTime();

	return iExtraLen;
}

void CInputMain::PrivateShopClose(LPCHARACTER ch)
{
	if (!ch || !ch->GetDesc())
		return;

	if (ch && ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Your logout has been canceled."));
		event_cancel(&ch->m_pkTimedEvent);

		return;
	}

	if (ch->IsStun() || ch->IsDead())
		return;

	if (!CheckTradeWindows(ch))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot close a personal shop while another window is open."));
		return;
	}

	if (!ch->IsPrivateShopOwner() || !ch->IsEditingPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You do not have an open personal shop."));
		return;
	}

	if (thecore_pulse() - ch->GetLastPrivateShopCloseTime() < PASSES_PER_SEC(10))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a few moments before closing your personal shop again."));
		return;
	}

	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_CLOSE;
	DWORD dwPID = ch->GetPlayerID();

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(DWORD));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&dwPID, sizeof(DWORD));

	ch->SetLastPrivateShopCloseTime();
}

void CInputMain::PrivateShopPanelOpen(LPCHARACTER ch)
{
	if (!ch || !ch->GetDesc())
		return;

	if (!CheckTradeWindows(ch))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot open a personal shop while another window is open."));
		return;
	}

	if (ch->IsEditingPrivateShop())
		return;

	ch->OpenPrivateShopPanel();
}

void CInputMain::PrivateShopPanelClose(LPCHARACTER ch)
{
	if (!ch)
		return;

	if (ch->IsStun() || ch->IsDead())
		return;

	ch->ClosePrivateShopPanel();

	// Set shop's state back to opened if it was left out in the modify state
	if (ch->CanModifyPrivateShop())
	{
		BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_MODIFY_REQUEST;
		DWORD dwPID = ch->GetPlayerID();
		db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(DWORD));
		db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
		db_clientdesc->Packet(&dwPID, sizeof(DWORD));
	}
}

int CInputMain::PrivateShopStart(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	const DWORD dwVID = *reinterpret_cast<const DWORD*>(c_pData);
	int iExtraLen = sizeof(DWORD);

	// No data has been passed
	if (uiBytes < iExtraLen)
		return -1;

	if (!ch || !ch->GetDesc())
		return iExtraLen;

	if (ch && ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Your logout has been canceled."));

		event_cancel(&ch->m_pkTimedEvent);

		return iExtraLen;
	}

	if (!CheckTradeWindows(ch))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot view a personal shop while having other trading windows open."));
		return iExtraLen;
	}

	LPPRIVATE_SHOP pPrivateShop = CPrivateShopManager::Instance().GetPrivateShopByVID(dwVID);
	if (!pPrivateShop)
		return iExtraLen;

	if (pPrivateShop->GetID() == ch->GetPlayerID())
	{
		if (!ch->IsEditingPrivateShop())
			ch->OpenPrivateShopPanel();

		return iExtraLen;
	}

	if (pPrivateShop == ch->GetViewingPrivateShop())
		return iExtraLen;

	if (ch->IsEditingPrivateShop())
		ch->ClosePrivateShopPanel();

	if (ch->GetViewingPrivateShop())
		ch->GetViewingPrivateShop()->RemoveShopViewer(ch);

	pPrivateShop->AddShopViewer(ch);

	return iExtraLen;
}

void CInputMain::PrivateShopEnd(LPCHARACTER ch)
{
	if (!ch)
		return;

	CPrivateShopManager::Instance().StopShopping(ch);
}

int CInputMain::PrivateShopBuy(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	const WORD wPos = *reinterpret_cast<const WORD*>(c_pData);
	int iExtraLen = sizeof(WORD);

	// No data has been passed
	if (uiBytes < iExtraLen)
		return -1;

	if (!ch || !ch->GetDesc())
		return iExtraLen;

	if (db_clientdesc->GetSocket() == INVALID_SOCKET)
		return iExtraLen;

	if (ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Your logout has been canceled."));
		event_cancel(&ch->m_pkTimedEvent);

		return iExtraLen;
	}

	if (thecore_pulse() - ch->GetLastPrivateShopBuyTime() < PASSES_PER_SEC(1))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a moment before buying from a personal shop again."));
		return iExtraLen;
	}

	if (ch->IsStun() || ch->IsDead())
		return iExtraLen;

	if (!ch->GetViewingPrivateShop())
		return iExtraLen;

	if (!ch->GetViewingPrivateShop()->GetItem(wPos))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot buy an item from your own personal shop."));
		return iExtraLen;
	}

	// Packet to db
	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_BUY_REQUEST;

	TPacketGDPrivateShopBuyRequest subPacket{};
	subPacket.dwCustomerPID = ch->GetPlayerID();
	subPacket.llGoldBalance = ch->GetGold();
#ifdef ENABLE_PRIVATE_SHOP_CHEQUE
	subPacket.dwChequeBalance = ch->GetCheque();
#else
	subPacket.dwChequeBalance = 0;
#endif
	subPacket.aSelectedItems[0].dwShopID = ch->GetViewingPrivateShop()->GetID();
	subPacket.aSelectedItems[0].wPos = wPos;
	subPacket.aSelectedItems[0].TPrice.llGold = ch->GetViewingPrivateShop()->GetItem(wPos)->GetGoldPrice();
	subPacket.aSelectedItems[0].TPrice.dwCheque = ch->GetViewingPrivateShop()->GetItem(wPos)->GetChequePrice();

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(TPacketGDPrivateShopBuyRequest));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&subPacket, sizeof(TPacketGDPrivateShopBuyRequest));

	ch->SetLastPrivateShopBuyTime();

	return iExtraLen;
}

void CInputMain::PrivateShopWithdraw(LPCHARACTER ch)
{
	if (!ch)
		return;

	if (ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Your logout has been canceled."));
		event_cancel(&ch->m_pkTimedEvent);

		return;
	}

	if (ch->IsStun() || ch->IsDead())
		return;

	if (!ch->IsPrivateShopOwner() || !ch->IsEditingPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You do not have an open personal shop."));
		return;
	}

	if (thecore_pulse() - ch->GetLastPrivateShopWithdrawTime() < PASSES_PER_SEC(10))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a few moments before withdrawing your personal shop again."));
		return;
	}

	if ((ch->GetPrivateShopTable()->llGold + ch->GetGold()) >= GOLD_MAX_MAX)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot exchange as you would exceed the maximum amount of Yang."));
		return;
	}

#ifdef ENABLE_PRIVATE_SHOP_CHEQUE
	if ((ch->GetPrivateShopTable()->dwCheque + ch->GetCheque()) > CHEQUE_MAX)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot exchange as you would exceed the maximum amount of Won."));
		return;
	}
#endif

	if (!ch->GetPrivateShopTable()->llGold && !ch->GetPrivateShopTable()->dwCheque)
		return;

	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_WITHDRAW_REQUEST;
	DWORD dwPID = ch->GetPlayerID();
	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(DWORD));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&dwPID, sizeof(DWORD));

	ch->SetLastPrivateShopWithdrawTime();
}

void CInputMain::PrivateShopModify(LPCHARACTER ch)
{
	if (!ch)
		return;

	if (ch->IsStun() || ch->IsDead())
		return;

	if (!ch->IsPrivateShopOwner() || !ch->IsEditingPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, "You do not have an open private shop.");
		return;
	}

	if (thecore_pulse() - ch->GetLastPrivateShopStateChangeTime() < PASSES_PER_SEC(1))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a moment before changing state of your personal shop again."));
		return;
	}

	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_MODIFY_REQUEST;
	DWORD dwPID = ch->GetPlayerID();
	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(DWORD));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&dwPID, sizeof(DWORD));

	ch->SetLastPrivateShopStateChangeTime();
}

int CInputMain::PrivateShopItemPriceChange(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	TPacketCGPrivateShopItemPriceChange* p = (TPacketCGPrivateShopItemPriceChange*)c_pData;

	int iExtraLen = sizeof(TPacketCGPrivateShopItemPriceChange);

	// No data has been passed
	if (uiBytes < iExtraLen)
		return -1;

	if (!ch)
		return iExtraLen;

	if (ch->IsStun() || ch->IsDead())
		return iExtraLen;

	if (ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Your logout has been canceled."));
		event_cancel(&ch->m_pkTimedEvent);

		return iExtraLen;
	}

	if (!CheckTradeWindows(ch))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot modify a personal shop while having other trading windows open."));
		return iExtraLen;
	}

	if (!ch->IsPrivateShopOwner() || !ch->IsEditingPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You do not have an open personal shop."));
		return iExtraLen;
	}

	if (!ch->CanModifyPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot manage personal shop's content while it is not in a modifying state."));
		return iExtraLen;
	}

	if (thecore_pulse() - ch->GetLastPrivateShopModifyTime() < PASSES_PER_SEC(1))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a moment before editing your personal shop's content again."));
		return iExtraLen;
	}

	const TPlayerPrivateShopItem* pPrivateShopItem = ch->GetPrivateShopItem(p->wPos);
	if (!pPrivateShopItem)
		return iExtraLen;

	if ((ch->GetPrivateShopTotalGold() + ch->GetGold() - pPrivateShopItem->TPrice.llGold + p->TPrice.llGold) >= GOLD_MAX_MAX)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The items you put up for sale must not exceed the permitted total value."));
		return iExtraLen;
	}

#ifdef ENABLE_PRIVATE_SHOP_CHEQUE
	if ((ch->GetPrivateShopTotalCheque() + ch->GetCheque() - pPrivateShopItem->TPrice.dwCheque + p->TPrice.dwCheque) > CHEQUE_MAX)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The items you put up for sale must not exceed the permitted total value."));
		return iExtraLen;
	}
#endif

	if (p->TPrice.llGold < 0)
	{
		sys_err("Player %u is trying to negatively manipulate price of the item", ch->GetPlayerID());
		return iExtraLen;
	}

	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_ITEM_PRICE_CHANGE_REQUEST;

	TPacketPrivateShopItemPriceChange subPacket{};
	subPacket.dwShopID = ch->GetPlayerID();
	subPacket.wPos = p->wPos;
	subPacket.TPrice.llGold = p->TPrice.llGold;
	subPacket.TPrice.dwCheque = p->TPrice.dwCheque;

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(TPacketPrivateShopItemPriceChange));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&subPacket, sizeof(TPacketPrivateShopItemPriceChange));

	ch->SetLastPrivateShopModifyTime();

	return iExtraLen;
}

int CInputMain::PrivateShopItemMove(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	TPacketCGPrivateShopItemMove* p = (TPacketCGPrivateShopItemMove*)c_pData;

	int iExtraLen = sizeof(TPacketCGPrivateShopItemMove);

	// No data has been passed
	if (uiBytes < iExtraLen)
		return -1;

	if (!ch)
		return iExtraLen;

	if (ch->IsStun() || ch->IsDead())
		return iExtraLen;

	if (!ch->CanHandleItem())
		return iExtraLen;

	if (ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Your logout has been canceled."));
		event_cancel(&ch->m_pkTimedEvent);

		return iExtraLen;
	}

	if (!CheckTradeWindows(ch))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot modify a personal shop while having other trading windows open."));
		return iExtraLen;
	}

	if (!ch->IsPrivateShopOwner() || !ch->IsEditingPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You do not have an open personal shop."));
		return iExtraLen;
	}

	if (!ch->CanModifyPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot manage personal shop's content while it is not in a modifying state."));
		return iExtraLen;
	}

	if (thecore_pulse() - ch->GetLastPrivateShopModifyTime() < PASSES_PER_SEC(1))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a moment before editing your personal shop's content again."));
		return iExtraLen;
	}

	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_ITEM_MOVE_REQUEST;

	TPacketPrivateShopItemMove subPacket{};
	subPacket.dwShopID = ch->GetPlayerID();
	subPacket.wPos = p->wPos;
	subPacket.wChangePos = p->wChangePos;

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(TPacketPrivateShopItemMove));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&subPacket, sizeof(TPacketPrivateShopItemMove));

	ch->SetLastPrivateShopModifyTime();

	return iExtraLen;
}

int CInputMain::PrivateShopItemCheckin(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	TPacketCGPrivateShopItemCheckin* p = (TPacketCGPrivateShopItemCheckin*)c_pData;

	int iExtraLen = sizeof(TPacketCGPrivateShopItemCheckin);

	// No data has been passed
	if (uiBytes < iExtraLen)
		return -1;

	if (!ch)
		return iExtraLen;

	if (ch->IsStun() || ch->IsDead())
		return iExtraLen;

	if (!ch->CanHandleItem())
		return iExtraLen;

	if (ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Your logout has been canceled."));
		event_cancel(&ch->m_pkTimedEvent);

		return iExtraLen;
	}

	if (!CheckTradeWindows(ch))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot modify a personal shop while having other trading windows open."));
		return iExtraLen;
	}

	if (!ch->IsPrivateShopOwner() || !ch->IsEditingPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You do not have an open personal shop."));
		return iExtraLen;
	}

	if (!ch->CanModifyPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot manage personal shop's content while it is not in a modifying state."));
		return iExtraLen;
	}

	if (thecore_pulse() - ch->GetLastPrivateShopModifyTime() < PASSES_PER_SEC(1))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a moment before editing your personal shop's content again."));
		return iExtraLen;
	}

	if ((ch->GetPrivateShopTotalGold() + ch->GetGold() + p->llGold) >= GOLD_MAX_MAX)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The items you put up for sale must not exceed the permitted total value."));
		return iExtraLen;
	}

#ifdef ENABLE_PRIVATE_SHOP_CHEQUE
	if ((ch->GetPrivateShopTotalCheque() + ch->GetCheque() + p->dwCheque) > CHEQUE_MAX)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The items you put up for sale must not exceed the permitted total value."));
		return iExtraLen;
	}
#endif

	if (p->llGold < 0)
	{
		sys_err("Player %u is trying to add an item with negative price", ch->GetPlayerID());
		return iExtraLen;
	}

	LPITEM pItem = ch->GetItem(p->TSrcPos);
	if (!pItem)
		return iExtraLen;

	if (!pItem->GetOwner() || ch != pItem->GetOwner())
	{
		sys_err("Player %u tried to add item %u that is not bound to him", ch->GetPlayerID(), pItem->GetID());
		return iExtraLen;
	}

	const TItemTable* pItemTable = pItem->GetProto();
	if (!pItemTable)
	{
		sys_err("Could not find an item table for an item at position: %d vnum: %d", p->TSrcPos, pItem->GetVnum());
		return false;
	}

	if (pItemTable && (IS_SET(pItemTable->dwAntiFlags, ITEM_ANTIFLAG_GIVE | ITEM_ANTIFLAG_MYSHOP)))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot sell Item-Shop items in a personal shop."));
		return false;
	}

	if (pItem->IsEquipped())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot sell equipped items in a personal shop."));
		return false;
	}

	if (pItem->isLocked())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot sell locked items in a personal shop."));
		return false;
	}

	// Flush delayed item data as we don't want it flushed later on after (if) its saved in private shop
	ITEM_MANAGER::Instance().FlushDelayedSave(pItem);

	TPlayerPrivateShopItem t;
	t.dwID = pItem->GetID();
	t.wPos = 0;
	t.dwCount = pItem->GetCount();
	t.dwVnum = pItem->GetOriginalVnum();
	memcpy(t.alSockets, pItem->GetSockets(), sizeof(t.alSockets));
	memcpy(t.aAttr, pItem->GetAttributes(), sizeof(t.aAttr));
	t.TPrice.llGold = p->llGold;
	t.TPrice.dwCheque = p->dwCheque;
	t.dwOwner = pItem->GetOwner()->GetPlayerID();
	t.tCheckin = time(0);
#ifdef ENABLE_PRIVATE_SHOP_CHANGE_LOOK
    t.dwTransmutationVnum = pItem->GetTransmutationVnum();
#endif
#ifdef ENABLE_PRIVATE_SHOP_REFINE_ELEMENT
    t.dwRefineElement = pItem->GetRefineElement();
#endif
#ifdef ENABLE_PRIVATE_SHOP_APPLY_RANDOM
    thecore_memcpy(t.aApplyRandom, pItem->GetRandomApplies(), sizeof(t.aApplyRandom));
#endif
#ifdef ENABLE_PET_GROWTH_SYSTEM
	if (pItem->GetPetGrowthSystem())
		t.pet = pItem->GetPetGrowthSystem()->GetPetCache();
	else
		t.pet = {};
#endif
	TPacketGDPrivateShopItemCheckin subPacket{};
	subPacket.dwShopID = ch->GetPlayerID();
	subPacket.TItem = t;
	subPacket.iPos = p->iDstPos;

	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKIN_REQUEST;
	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(TPacketGDPrivateShopItemCheckin));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&subPacket, sizeof(TPacketGDPrivateShopItemCheckin));

	ch->SetLastPrivateShopModifyTime();

	return iExtraLen;
}

int CInputMain::PrivateShopItemCheckout(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	TPacketCGPrivateShopItemCheckout* p = (TPacketCGPrivateShopItemCheckout*)c_pData;

	int iExtraLen = sizeof(TPacketCGPrivateShopItemCheckout);

	// No data has been passed
	if (uiBytes < iExtraLen)
		return -1;

	if (!ch)
		return iExtraLen;

	if (ch->IsStun() || ch->IsDead())
		return iExtraLen;

	if (!ch->CanHandleItem())
		return iExtraLen;

	if (!CheckTradeWindows(ch))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot modify a personal shop while having other trading windows open."));
		return iExtraLen;
	}

	if (ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Your logout has been canceled."));
		event_cancel(&ch->m_pkTimedEvent);

		return iExtraLen;
	}

	if (!ch->IsPrivateShopOwner() || !ch->IsEditingPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You do not have an open personal shop."));
		return iExtraLen;
	}

	if (!ch->CanModifyPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot manage personal shop's content while it is not in a modifying state."));
		return iExtraLen;
	}

	if (thecore_pulse() - ch->GetLastPrivateShopModifyTime() < PASSES_PER_SEC(1))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a moment before editing your personal shop's content again."));
		return iExtraLen;
	}

	const TPlayerPrivateShopItem* c_pShopItem = ch->GetPrivateShopItem(p->wSrcPos);
	if (!c_pShopItem)
	{
		sys_err("Cannot find item on position %d pid %u", p->wSrcPos, ch->GetPlayerID());
		return iExtraLen;
	}

	const TItemTable* pItemTable = ITEM_MANAGER::Instance().GetTable(c_pShopItem->dwVnum);
	if (!pItemTable)
	{
		sys_err("Cannot find item table for item vnum %d", c_pShopItem->dwVnum);
		return iExtraLen;
	}

	BYTE bWindow = RESERVED_WINDOW;
	LPITEM pFakeItem = ITEM_MANAGER::Instance().CreateItem(c_pShopItem->dwVnum);
	int iPos = GetEmptyInventory(ch, pFakeItem);

	// Find a position if none was specified or the specified one is invalid
	if (pItemTable->bType == ITEM_DS)
	{
		if (p->iDstPos < 0 || !ch->IsEmptyItemGrid(TItemPos(DRAGON_SOUL_INVENTORY, p->iDstPos), pItemTable->bSize))
		{
			if (iPos < 0)
			{
				ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough space in your inventory."));
				M2_DESTROY_ITEM(pFakeItem);
				return iExtraLen;
			}

			p->iDstPos = iPos;
		}

		bWindow = DRAGON_SOUL_INVENTORY;
	}
	else
	{
#ifdef ENABLE_PRIVATE_SHOP_SPECIAL_INV
		if (p->iDstPos >= SKILL_BOOK_INVENTORY_SLOT_START && p->iDstPos < GIFT_BOX_INVENTORY_SLOT_END)
		{
			TItemPos targetCell(INVENTORY, p->iDstPos);

			if (pFakeItem->IsSkillBook())
			{
				if (!targetCell.IsSkillBookInventoryPosition())
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot move this item to this window."));
					M2_DESTROY_ITEM(pFakeItem);
					return iExtraLen;
				}
			}
			else if (pFakeItem->IsUpgradeItem())
			{
				if (!targetCell.IsUpgradeItemsInventoryPosition())
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot move this item to this window."));
					M2_DESTROY_ITEM(pFakeItem);
					return iExtraLen;
				}
			}
			else if (pFakeItem->IsStone())
			{
				if (!targetCell.IsStoneInventoryPosition())
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot move this item to this window."));
					M2_DESTROY_ITEM(pFakeItem);
					return iExtraLen;
				}
			}
			else if (pFakeItem->IsGiftBox())
			{
				if (!targetCell.IsBoxInventoryPosition())
				{
					ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot move this item to this window."));
					M2_DESTROY_ITEM(pFakeItem);
					return iExtraLen;
				}
			}
			else
			{
				ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot move this item to this window."));
				M2_DESTROY_ITEM(pFakeItem);
				return iExtraLen;
			}
		}
#endif

		if (p->iDstPos < 0 || !ch->IsEmptyItemGrid(TItemPos(INVENTORY, p->iDstPos), pItemTable->bSize))
		{
			if (iPos < 0)
			{
				ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You don't have enough space in your inventory."));
				M2_DESTROY_ITEM(pFakeItem);
				return iExtraLen;
			}

			p->iDstPos = iPos;
		}

		bWindow = INVENTORY;
	}

	M2_DESTROY_ITEM(pFakeItem);

	// Packet to db
	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_ITEM_CHECKOUT_REQUEST;

	TPacketGDPrivateShopItemCheckout subPacket{};
	subPacket.dwPID = ch->GetPlayerID();
	subPacket.wSrcPos = p->wSrcPos;
	subPacket.TDstPos.cell = p->iDstPos;
	subPacket.TDstPos.window_type = bWindow;
	subPacket.TItem = *c_pShopItem;

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(TPacketGDPrivateShopItemCheckout));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&subPacket, sizeof(TPacketGDPrivateShopItemCheckout));

	ch->SetLastPrivateShopModifyTime();

	return iExtraLen;
}

int CInputMain::PrivateShopTitleChange(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	const char* c_szTitle = c_pData;
	int iExtraLen = TITLE_MAX_LEN + 1;

	// No data has been passed
	if (uiBytes < iExtraLen)
		return -1;

	if (!ch)
		return iExtraLen;

	if (ch->IsStun() || ch->IsDead())
		return iExtraLen;

	if (!ch->IsPrivateShopOwner() || !ch->IsEditingPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You do not have an open personal shop."));
		return iExtraLen;
	}

	if (!ch->CanModifyPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot manage personal shop's content while it is not in a modifying state."));
		return iExtraLen;
	}

	if (strlen(c_szTitle) < TITLE_MIN_LEN)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("The entered name is too short."));
		return iExtraLen;
	}

	if (thecore_pulse() - ch->GetLastPrivateShopModifyTime() < PASSES_PER_SEC(1))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a moment before editing your personal shop's content again."));
		return iExtraLen;
	}

	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_TITLE_CHANGE_REQUEST;

	TPacketPrivateShopTitleChange subPacket{};
	subPacket.dwPID = ch->GetPlayerID();
	strncpy(subPacket.szTitle, c_szTitle, TITLE_MAX_LEN);

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(TPacketPrivateShopTitleChange));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&subPacket, sizeof(TPacketPrivateShopTitleChange));

	ch->SetLastPrivateShopModifyTime();

	return iExtraLen;
}

void CInputMain::PrivateShopWarpRequest(LPCHARACTER ch)
{
	if (!ch || !ch->GetDesc())
		return;

	if (ch->IsStun() || ch->IsDead())
		return;

	if (ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Your logout has been canceled."));
		event_cancel(&ch->m_pkTimedEvent);

		return;
	}

	if (!ch->IsPrivateShopOwner())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You do not have an open personal shop."));
		return;
	}

	TPrivateShop* pTable = ch->GetPrivateShopTable();
	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_WARP_REQUEST;

	TPacketGDPrivateShopWarpReq subPacket{};
	subPacket.dwPID = ch->GetPlayerID();
	subPacket.dwMapIndex = pTable->lMapIndex;
	subPacket.wListenPort = pTable->wPort;
	subPacket.bChannel = pTable->bChannel;

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(TPacketGDPrivateShopWarpReq));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&subPacket, sizeof(TPacketGDPrivateShopWarpReq));
}

int CInputMain::PrivateShopSlotUnlockRequest(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	const WORD wPos = *reinterpret_cast<const WORD*>(c_pData);

	int iExtraLen = sizeof(WORD);

	// No data has been passed
	if (uiBytes < iExtraLen)
		return -1;

	if (!ch || !ch->GetDesc())
		return iExtraLen;

	if (ch->IsStun() || ch->IsDead())
		return iExtraLen;

	if (!ch->CanHandleItem())
		return iExtraLen;

	if (!CheckTradeWindows(ch))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot modify a personal shop while having other trading windows open."));
		return iExtraLen;
	}

	if (ch->m_pkTimedEvent)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Your logout has been canceled."));
		event_cancel(&ch->m_pkTimedEvent);

		return iExtraLen;
	}

	if (ch->IsEditingPrivateShop() && !ch->CanModifyPrivateShop())
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot manage personal shop's content while it is not in a modifying state."));
		return iExtraLen;
	}

	LPITEM pUnlockItem = ch->GetItem(TItemPos(INVENTORY, wPos));

	if (!pUnlockItem || pUnlockItem->GetVnum() != PRIVATE_SHOP_SLOT_UNLOCK_ITEM)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot unlock a slot on your personal shop without Personal Shop Key."));
		return iExtraLen;
	}

	WORD wUnlockedSlots = ch->GetPoint(POINT_PRIVATE_SHOP_UNLOCKED_SLOT);
	int iAvailableSlots = PRIVATE_SHOP_LOCKED_SLOT_MAX_NUM - wUnlockedSlots;

	if (!iAvailableSlots)
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot unlock any more slots."));
		return iExtraLen;
	}

	// If player has an open shop, await a reply from the db
	if (ch->IsPrivateShopOwner())
	{
		BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_SLOT_UNLOCK_REQUEST;
		TPacketGDPrivateShopSlotUnlockReq subPacket{};
		subPacket.dwPID = ch->GetPlayerID();
		subPacket.wCount = pUnlockItem->GetValue(0);

		db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(TPacketGDPrivateShopSlotUnlockReq));
		db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
		db_clientdesc->Packet(&subPacket, sizeof(TPacketGDPrivateShopSlotUnlockReq));
	}
	else
	{
		// Prevent overflow
		WORD wIncreasedSlotCount = std::min<WORD>(pUnlockItem->GetValue(0), iAvailableSlots);
		ch->PointChange(POINT_PRIVATE_SHOP_UNLOCKED_SLOT, wIncreasedSlotCount, true);

		pUnlockItem->SetCount(pUnlockItem->GetCount() - 1);
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You have increased available space on your personal shop."));
	}

	return iExtraLen;
}

void CInputMain::PrivateShopSearchClose(LPCHARACTER ch)
{
	if (!ch)
		return;

	ch->CloseShopSearch();
}

int CInputMain::PrivateShopSearch(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	TPacketCGPrivateShopSearch* p = (TPacketCGPrivateShopSearch*)c_pData;

	int iExtraLen = sizeof(TPacketCGPrivateShopSearch);

	// No data has been passed
	if (uiBytes < iExtraLen)
		return -1;

	if (!ch)
		return iExtraLen;

	if (ch->IsStun() || ch->IsDead())
		return iExtraLen;

	if (thecore_pulse() - ch->GetLastPrivateShopSearchTime() < PASSES_PER_SEC(2))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a moment before searching other personal shops again."));
		return iExtraLen;
	}

	CPrivateShopManager::Instance().SearchItem(ch->GetDesc(), p->Filter, p->bUseFilter);

	TPacketGGPrivateShopItemSearch packet{};
	packet.bHeader = HEADER_GG_PRIVATE_SHOP_ITEM_SEARCH;
	packet.dwCustomerID = ch->GetPlayerID();
	packet.dwCustomerPort = p2p_port;
	packet.bUseFilter = p->bUseFilter;
	memcpy(&packet.Filter, &p->Filter, sizeof(packet.Filter));

	P2P_MANAGER::Instance().Send(&packet, sizeof(TPacketGGPrivateShopItemSearch));

	ch->SetLastPrivateShopSearchTime();
	return iExtraLen;
}

int CInputMain::PrivateShopSearchBuy(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	TPacketCGPrivateShopSearchBuy* p = (TPacketCGPrivateShopSearchBuy*)c_pData;

	int iExtraLen = sizeof(TPacketCGPrivateShopSearchBuy);

	// No data has been passed
	if (uiBytes < iExtraLen)
		return -1;

	if (!ch)
		return iExtraLen;

	if (db_clientdesc->GetSocket() == INVALID_SOCKET)
		return iExtraLen;

	if (ch->IsStun() || ch->IsDead())
		return iExtraLen;

	if (!ch->IsShopSearch())
	{
		sys_err("Player %u is tryint to buy an item with no window opened", ch->GetPlayerID());
		return iExtraLen;
	}

	for (BYTE i = 0; i < SELECTED_ITEM_MAX_NUM; ++i)
	{
		if (p->aSelectedItems[i].dwShopID == ch->GetPlayerID())
		{
			ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("You cannot buy an item from your own personal shop."));
			return iExtraLen;
		}
	}

	if (thecore_pulse() - ch->GetLastPrivateShopBuyTime() < PASSES_PER_SEC(1))
	{
		ch->ChatPacket(CHAT_TYPE_INFO, LC_TEXT("Please wait a moment before buying from a personal shop again."));
		return iExtraLen;
	}

	// Packet to db
	BYTE bSubHeader = PRIVATE_SHOP_GD_SUBHEADER_BUY_REQUEST;

	TPacketGDPrivateShopBuyRequest subPacket{};
	subPacket.dwCustomerPID = ch->GetPlayerID();
	subPacket.llGoldBalance = ch->GetGold();
#ifdef ENABLE_PRIVATE_SHOP_CHEQUE
	subPacket.dwChequeBalance = ch->GetCheque();
#else
	subPacket.dwChequeBalance = 0;
#endif

	for (BYTE i = 0; i < SELECTED_ITEM_MAX_NUM; ++i)
	{
		if (!p->aSelectedItems[i].dwShopID)
			break;

		subPacket.aSelectedItems[i].dwShopID = p->aSelectedItems[i].dwShopID;
		subPacket.aSelectedItems[i].wPos = p->aSelectedItems[i].wPos;
		subPacket.aSelectedItems[i].TPrice.llGold = p->aSelectedItems[i].TPrice.llGold;
		subPacket.aSelectedItems[i].TPrice.dwCheque = p->aSelectedItems[i].TPrice.dwCheque;
	}

	db_clientdesc->DBPacketHeader(HEADER_GD_PRIVATE_SHOP, ch->GetDesc()->GetHandle(), sizeof(BYTE) + sizeof(TPacketGDPrivateShopBuyRequest));
	db_clientdesc->Packet(&bSubHeader, sizeof(BYTE));
	db_clientdesc->Packet(&subPacket, sizeof(TPacketGDPrivateShopBuyRequest));

	ch->SetLastPrivateShopBuyTime();
	return iExtraLen;
}
void CInputMain::PrivateShopMarketItemPriceDataRequest(LPCHARACTER ch)
{
	if (!ch)
		return;

	CPrivateShopManager::Instance().SendMarketItemPriceData(ch);
}

int CInputMain::PrivateShopMarketItemPriceRequest(LPCHARACTER ch, const char* c_pData, size_t uiBytes)
{
	DWORD dwVnum = *(DWORD*)c_pData;

	int iExtraLen = sizeof(DWORD);

	// No data has been passed
	if (uiBytes < iExtraLen)
		return -1;

	if (!ch)
		return iExtraLen;

	CPrivateShopManager::Instance().SendMarketItemPrice(ch, dwVnum);
	return iExtraLen;
}
#endif




#ifdef __AURA_SYSTEM__
size_t GetAuraSubPacketLength(const EPacketCGAuraSubHeader& SubHeader)
{
	switch (SubHeader)
	{
	case AURA_SUBHEADER_CG_REFINE_CHECKIN:
		return sizeof(TSubPacketCGAuraRefineCheckIn);
	case AURA_SUBHEADER_CG_REFINE_CHECKOUT:
		return sizeof(TSubPacketCGAuraRefineCheckOut);
	case AURA_SUBHEADER_CG_REFINE_ACCEPT:
		return sizeof(TSubPacketCGAuraRefineAccept);
	case AURA_SUBHEADER_CG_REFINE_CANCEL:
		return 0;
	}

	return 0;
}

int CInputMain::Aura(LPCHARACTER ch, const char* data, size_t uiBytes)
{
	if (uiBytes < sizeof(TPacketCGAura))
		return -1;

	const TPacketCGAura* pinfo = reinterpret_cast<const TPacketCGAura*>(data);
	const char* c_pData = data + sizeof(TPacketCGAura);

	uiBytes -= sizeof(TPacketCGAura);

	const EPacketCGAuraSubHeader SubHeader = static_cast<EPacketCGAuraSubHeader>(pinfo->bSubHeader);
	const size_t SubPacketLength = GetAuraSubPacketLength(SubHeader);
	if (uiBytes < SubPacketLength)
	{
		sys_err("invalid aura subpacket length (sublen %d size %u buffer %u)", SubPacketLength, sizeof(TPacketCGAura), uiBytes);
		return -1;
	}

	switch (SubHeader)
	{
		case AURA_SUBHEADER_CG_REFINE_CHECKIN:
			{
				const TSubPacketCGAuraRefineCheckIn* sp = reinterpret_cast<const TSubPacketCGAuraRefineCheckIn*>(c_pData);
				ch->AuraRefineWindowCheckIn(sp->byAuraRefineWindowType, sp->AuraCell, sp->ItemCell);
			}
			return SubPacketLength;
		case AURA_SUBHEADER_CG_REFINE_CHECKOUT:
			{
				const TSubPacketCGAuraRefineCheckOut* sp = reinterpret_cast<const TSubPacketCGAuraRefineCheckOut*>(c_pData);
				ch->AuraRefineWindowCheckOut(sp->byAuraRefineWindowType, sp->AuraCell);
			}
			return SubPacketLength;
		case AURA_SUBHEADER_CG_REFINE_ACCEPT:
			{
				const TSubPacketCGAuraRefineAccept* sp = reinterpret_cast<const TSubPacketCGAuraRefineAccept*>(c_pData);
				ch->AuraRefineWindowAccept(sp->byAuraRefineWindowType);
			}
			return SubPacketLength;
		case AURA_SUBHEADER_CG_REFINE_CANCEL:
			{
				ch->AuraRefineWindowClose();
			}
			return SubPacketLength;
	}

	return 0;
}
#endif

#ifdef __ENABLE_BIOLOGIST_RENEWAL_SYSTEM__
void CInputMain::RecvBiologistPacket(LPCHARACTER ch, const char* c_pData)
{
	const TPacketCGBiologist* p = reinterpret_cast<const TPacketCGBiologist*>(c_pData);
	switch (p->bySubHeader)
	{
		case BIOLOGIST_SUBHEADER_CG_OPEN:
			CBiologist::instance().BiologistOpenPacket(ch);
			break;
		case BIOLOGIST_SUBHEADER_CG_PROVIDES:
			CBiologist::instance().BiologistProvidesMaterialPacket(ch, p->byIsElixirUse, p->byIsBookTimeUse);
			break;
		case BIOLOGIST_SUBHEADER_CG_SELECTIVE:
			CBiologist::instance().BiologistChosenAffectPacket(ch, p->byChosenAffect);
			break;
		case BIOLOGIST_SUBHEADER_CG_TIME:
			CBiologist::instance().BiologistDecreaseTime(ch, p->byDecreaseTimeIndex);
			break;
		default:
			break;
	}
}
#endif



#ifdef __ENABLE_PREMIUM_PLAYERS__
void CInputMain::RecvPremiumPlayersPacket(LPCHARACTER ch, const char* c_pData)
{
	const TPacketCGPremiumPlayers* p = reinterpret_cast<const TPacketCGPremiumPlayers*>(c_pData);
	switch (p->bySubHeader)
	{
		case PREMIUM_PLAYERS_SUBHEADER_CG_OPEN:
			ch->PremiumPlayersOpenPacket();
			break;
		case PREMIUM_PLAYERS_SUBHEADER_CG_LIST:
			ch->PremiumPlayersListPacket();
			break;
		case PREMIUM_PLAYERS_SUBHEADER_CG_ACTIVATE:
			ch->PremiumPlayersActivatePacket();
			break;
		default:
			break;
	}
}
#endif


#ifdef __ENABLE_INVENTORY_PROTECTED_SYSTEM__
void CInputMain::RecvActivateProtectedSystem(LPCHARACTER ch, const char* c_pData)
{
	const SPacketCGInventoryProtected* kRecv = reinterpret_cast<const SPacketCGInventoryProtected*>(c_pData);
	switch (kRecv->bySubHeader)
	{
		case SUBHEADER_CG_INVENTORY_PROTECTED_ACTIVATE:
			ch->ActivateProtectedSystem(kRecv->szPasswordNow, kRecv->bActivated);
			break;
		case SUBHEADER_CG_INVENTORY_PROTECTED_PASSWORD_CHANGE:
			ch->ChangeProtectedSystemPassword(kRecv->szPasswordNow, kRecv->szPasswordNew);
			break;
		default:
			break;
	}
}
#endif


#ifdef ENABLE_SWITCHBOT
int CInputMain::Switchbot(LPCHARACTER ch, const char* data, size_t uiBytes)
{
	const TPacketCGSwitchbot* p = reinterpret_cast<const TPacketCGSwitchbot*>(data);

	if (uiBytes < sizeof(TPacketCGSwitchbot))
	{
		return -1;
	}

	const char* c_pData = data + sizeof(TPacketCGSwitchbot);
	uiBytes -= sizeof(TPacketCGSwitchbot);

	switch (p->subheader)
	{
	case SUBHEADER_CG_SWITCHBOT_START:
	{
		size_t extraLen = sizeof(TSwitchbotAttributeAlternativeTable) * SWITCHBOT_ALTERNATIVE_COUNT;
		if (uiBytes < extraLen)
		{
			return -1;
		}

		std::vector<TSwitchbotAttributeAlternativeTable> vec_alternatives;

		for (BYTE alternative = 0; alternative < SWITCHBOT_ALTERNATIVE_COUNT; ++alternative)
		{
			const TSwitchbotAttributeAlternativeTable* pAttr = reinterpret_cast<const TSwitchbotAttributeAlternativeTable*>(c_pData);
			c_pData += sizeof(TSwitchbotAttributeAlternativeTable);

			vec_alternatives.emplace_back(*pAttr);
		}

		CSwitchbotManager::Instance().Start(ch->GetPlayerID(), p->slot, vec_alternatives);
		return extraLen;
	}

	case SUBHEADER_CG_SWITCHBOT_STOP:
	{
		CSwitchbotManager::Instance().Stop(ch->GetPlayerID(), p->slot);
		return 0;
	}
	}

	return 0;
}
#endif

void CInputMain::RequestEventQuest(LPCHARACTER ch, const void* c_pData)
{
	TPacketCGRequestEventQuest* p = (TPacketCGRequestEventQuest*)c_pData;

	if (ch && ch->GetDesc())
		quest::CQuestManager::instance().RequestEventQuest(p->szName, ch->GetPlayerID());
}

#ifdef __EVENT_MANAGER__
void CInputMain::RequestEventData(LPCHARACTER ch, const char* c_pData)
{
	if (ch && ch->GetDesc())
	{
		const TPacketCGRequestEventData* p = reinterpret_cast<const TPacketCGRequestEventData*>(c_pData);
		CEventManager::Instance().SendEventInfo(ch, p->bMonth);
	}
}
#endif

