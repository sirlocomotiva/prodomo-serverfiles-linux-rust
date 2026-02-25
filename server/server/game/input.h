#ifndef __INC_METIN_II_GAME_INPUT_PROCESSOR__
#define __INC_METIN_II_GAME_INPUT_PROCESSOR__
#include "packet_info.h"
enum
{
	INPROC_CLOSE,
	INPROC_HANDSHAKE,
	INPROC_LOGIN,
	INPROC_MAIN,
	INPROC_DEAD,
	INPROC_DB,
	INPROC_UDP,
	INPROC_P2P,
	INPROC_AUTH,
};
void LoginFailure(LPDESC d, const char * c_pszStatus);
extern void SendShout(const char * szText, BYTE bEmpire);
class CInputProcessor
{
	public:
		CInputProcessor();
		virtual ~CInputProcessor() {};
		virtual bool Process(LPDESC d, const void * c_pvOrig, int iBytes, int & r_iBytesProceed);
		virtual BYTE GetType() = 0;
		void BindPacketInfo(CPacketInfo * pPacketInfo);
		void Pong(LPDESC d);
		void Handshake(LPDESC d, const char * c_pData);
		void Version(LPCHARACTER ch, const char* c_pData);
	protected:
		virtual int	Analyze(LPDESC d, BYTE bHeader, const char * c_pData) = 0;
		CPacketInfo * m_pPacketInfo;
		int	m_iBufferLeft;
		CPacketInfoCG 	m_packetInfoCG;
};
class CInputClose : public CInputProcessor
{
	public:
		virtual BYTE	GetType() { return INPROC_CLOSE; }
	protected:
		virtual int	Analyze(LPDESC d, BYTE bHeader, const char * c_pData) { return m_iBufferLeft; }
};
class CInputHandshake : public CInputProcessor
{
	public:
		CInputHandshake();
		virtual ~CInputHandshake();
		virtual BYTE	GetType() { return INPROC_HANDSHAKE; }
	protected:
		virtual int	Analyze(LPDESC d, BYTE bHeader, const char * c_pData);
	protected:
		void		GuildMarkLogin(LPDESC d, const char* c_pData);
		CPacketInfo *	m_pMainPacketInfo;
};
class CInputLogin : public CInputProcessor
{
	public:
		virtual BYTE	GetType() { return INPROC_LOGIN; }
	protected:
		virtual int	Analyze(LPDESC d, BYTE bHeader, const char * c_pData);
	protected:
		void		Login(LPDESC d, const char * data);
		void		LoginByKey(LPDESC d, const char * data);
		void		CharacterSelect(LPDESC d, const char * data);
		void		CharacterCreate(LPDESC d, const char * data);
		void		CharacterDelete(LPDESC d, const char * data);
		void		Entergame(LPDESC d, const char * data);
		void		Empire(LPDESC d, const char * c_pData);
		void		GuildMarkCRCList(LPDESC d, const char* c_pData);
		void		GuildMarkIDXList(LPDESC d, const char* c_pData);
		void		GuildMarkUpload(LPDESC d, const char* c_pData);
		int			GuildSymbolUpload(LPDESC d, const char* c_pData, size_t uiBytes);
		void		GuildSymbolCRC(LPDESC d, const char* c_pData);
		void		ChangeName(LPDESC d, const char * data);
};
class CInputMain : public CInputProcessor
{
	public:
		virtual BYTE	GetType() { return INPROC_MAIN; }
		int			Aura(LPCHARACTER ch, const char* data, size_t uiBytes);
	protected:
		virtual int	Analyze(LPDESC d, BYTE bHeader, const char * c_pData);
	protected:
		void		Attack(LPCHARACTER ch, const BYTE header, const char* data);
		int			Whisper(LPCHARACTER ch, const char * data, size_t uiBytes);
		void		InventoryExpansion(LPCHARACTER ch, const char * data);
		int			Chat(LPCHARACTER ch, const char * data, size_t uiBytes);
		void		ItemUse(LPCHARACTER ch, const char * data);
		void		ItemDrop(LPCHARACTER ch, const char * data);
		void		ItemDrop2(LPCHARACTER ch, const char * data);
		void		ItemDestroy(LPCHARACTER ch, const char * data);
		void		ItemMove(LPCHARACTER ch, const char * data);
		void		ItemPickup(LPCHARACTER ch, const char * data);
		void		ItemToItem(LPCHARACTER ch, const char * pcData);
		void		QuickslotAdd(LPCHARACTER ch, const char * data);
		void		QuickslotDelete(LPCHARACTER ch, const char * data);
		void		QuickslotSwap(LPCHARACTER ch, const char * data);
		int			Shop(LPCHARACTER ch, const char * data, size_t uiBytes);
		int			FishEvent(LPCHARACTER ch, const char * data, size_t uiBytes);
		void		OnClick(LPCHARACTER ch, const char * data);
		void		Exchange(LPCHARACTER ch, const char * data);
		void		Position(LPCHARACTER ch, const char * data);
		void		Move(LPCHARACTER ch, const char * data);
		int			SyncPosition(LPCHARACTER ch, const char * data, size_t uiBytes);
		void		FlyTarget(LPCHARACTER ch, const char * pcData, BYTE bHeader);
		void		UseSkill(LPCHARACTER ch, const char * pcData);
		void		ScriptAnswer(LPCHARACTER ch, const void * pvData);
		void		ScriptButton(LPCHARACTER ch, const void * pvData);
		void		ScriptSelectItem(LPCHARACTER ch, const void * pvData);
		void		QuestInputString(LPCHARACTER ch, const void * pvData);
		void		QuestConfirm(LPCHARACTER ch, const void* pvData);
		void		RequestEventQuest(LPCHARACTER ch, const void* pvData);
		void		Target(LPCHARACTER ch, const char * pcData);
		void		Warp(LPCHARACTER ch, const char * pcData);
		void		SafeboxCheckin(LPCHARACTER ch, const char * c_pData);
		void		SafeboxCheckout(LPCHARACTER ch, const char * c_pData, bool bMall);
		void		SafeboxItemMove(LPCHARACTER ch, const char * data);
		int			Messenger(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		void 		PartyInvite(LPCHARACTER ch, const char * c_pData);
		void 		PartyInviteAnswer(LPCHARACTER ch, const char * c_pData);
		void		PartyRemove(LPCHARACTER ch, const char * c_pData);
		void		PartySetState(LPCHARACTER ch, const char * c_pData);
		void		PartyUseSkill(LPCHARACTER ch, const char * c_pData);
		void		PartyParameter(LPCHARACTER ch, const char * c_pData);
		int			Guild(LPCHARACTER ch, const char * data, size_t uiBytes);
		void		AnswerMakeGuild(LPCHARACTER ch, const char* c_pData);
		void		Fishing(LPCHARACTER ch, const char* c_pData);
		void		ItemGive(LPCHARACTER ch, const char* c_pData);
		void		Hack(LPCHARACTER ch, const char * c_pData);
		int			MyShop(LPCHARACTER ch, const char * c_pData, size_t uiBytes);
		void 		CubeRenewalSend(LPCHARACTER ch, const char* data);
		void 		RecvPremiumPlayersPacket(LPCHARACTER ch, const char* c_pData);
		void 		RecvBiologistPacket(LPCHARACTER ch, const char* c_pData);
		void 		RecvActivateProtectedSystem(LPCHARACTER ch, const char* c_pData);
		void		Refine(LPCHARACTER ch, const char* c_pData);
		void 		GayaSystemSend(LPCHARACTER ch, const char * data);
		void 		RefineElement(LPCHARACTER ch, const char* c_pData);
		void		ChangeLook(LPCHARACTER pkChar, const char* c_pData);
		void		Sash(LPCHARACTER pkChar, const char* c_pData);
		void		Roulette(LPCHARACTER ch, const char* c_pData);
		void		RequestEventData(LPCHARACTER ch, const char* c_pData);
		int			Switchbot(LPCHARACTER ch, const char* data, size_t uiBytes);
		void 		ChangeLanguage(LPCHARACTER ch, BYTE bLanguage);
		void 		WhisperDetails(LPCHARACTER ch, const char* c_pData);
		void 		WorldBoss(const LPCHARACTER c_lpChar, const char* c_pszData);
		void 		DailyGift(LPCHARACTER ch, const char* c_pData);
		void 		Attr67Add(LPCHARACTER ch, const char* c_pData);
		void		TargetInfoLoad(LPCHARACTER ch, const char* c_pData);
		int			PrivateShopBuild(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		void		PrivateShopClose(LPCHARACTER ch);
		void		PrivateShopPanelOpen(LPCHARACTER ch);
		void		PrivateShopPanelClose(LPCHARACTER ch);
		int			PrivateShopStart(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		void		PrivateShopEnd(LPCHARACTER ch);
		int			PrivateShopBuy(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		void		PrivateShopWithdraw(LPCHARACTER ch);
		void		PrivateShopModify(LPCHARACTER ch);
		int			PrivateShopItemPriceChange(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		int			PrivateShopItemMove(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		int			PrivateShopItemCheckin(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		int			PrivateShopItemCheckout(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		int			PrivateShopTitleChange(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		void		PrivateShopWarpRequest(LPCHARACTER ch);
		int			PrivateShopSlotUnlockRequest(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		void		PrivateShopSearchClose(LPCHARACTER ch);
		int			PrivateShopSearch(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		int			PrivateShopSearchBuy(LPCHARACTER ch, const char* c_pData, size_t uiBytes);
		void		PrivateShopMarketItemPriceDataRequest(LPCHARACTER ch);
		int			PrivateShopMarketItemPriceRequest(LPCHARACTER ch, const char* c_pData, size_t uiBytes);

};
class CInputDead : public CInputMain
{
	public:
		virtual BYTE	GetType() { return INPROC_DEAD; }

	protected:
		virtual int	Analyze(LPDESC d, BYTE bHeader, const char * c_pData);
};
class CInputDB : public CInputProcessor
{
public:
	virtual bool Process(LPDESC d, const void * c_pvOrig, int iBytes, int & r_iBytesProceed);
	virtual BYTE GetType() { return INPROC_DB; }
protected:
	virtual int	Analyze(LPDESC d, BYTE bHeader, const char * c_pData);
protected:
	void		MapLocations(const char * c_pData);
	void		LoginSuccess(DWORD dwHandle, const char *data);
	void		PlayerCreateFailure(LPDESC d, BYTE bType);	// 0 = �Ϲ� ���� 1 = �̹� ����
	void		PlayerDeleteSuccess(LPDESC d, const char * data);
	void		PlayerDeleteFail(LPDESC d);
	void		PlayerLoad(LPDESC d, const char* data);
	void		PlayerCreateSuccess(LPDESC d, const char * data);
	void		Boot(const char* data);
	void		QuestLoad(LPDESC d, const char * c_pData);
	void		SafeboxLoad(LPDESC d, const char * c_pData);
	void		SafeboxChangeSize(LPDESC d, const char * c_pData);
	void		SafeboxWrongPassword(LPDESC d);
	void		SafeboxChangePasswordAnswer(LPDESC d, const char* c_pData);
	void		MallLoad(LPDESC d, const char * c_pData);
	void		EmpireSelect(LPDESC d, const char * c_pData);
	void		P2P(const char * c_pData);
	void		ItemLoad(LPDESC d, const char * c_pData);
	void		AffectLoad(LPDESC d, const char * c_pData);
	void		GuildLoad(const char * c_pData);
	void		GuildSkillUpdate(const char* c_pData);
	void		GuildSkillRecharge();
	void		GuildExpUpdate(const char* c_pData);
	void		GuildAddMember(const char* c_pData);
	void		GuildRemoveMember(const char* c_pData);
	void		GuildChangeGrade(const char* c_pData);
	void		GuildChangeMemberData(const char* c_pData);
	void		GuildDisband(const char* c_pData);
	void		GuildLadder(const char* c_pData);
	void		GuildWar(const char* c_pData);
	void		GuildWarScore(const char* c_pData);
	void		GuildSkillUsableChange(const char* c_pData);
	void		GuildMoneyChange(const char* c_pData);
	void		GuildWithdrawMoney(const char* c_pData);
	void		GuildWarReserveAdd(TGuildWarReserve * p);
	void		GuildWarReserveUpdate(TGuildWarReserve * p);
	void		GuildWarReserveDelete(DWORD dwID);
	void		GuildWarBet(TPacketGDGuildWarBet * p);
	void		GuildChangeMaster(TPacketChangeGuildMaster* p);
	void		LoginAlready(LPDESC d, const char * c_pData);
	void		PartyCreate(const char* c_pData);
	void		PartyDelete(const char* c_pData);
	void		PartyAdd(const char* c_pData);
	void		PartyRemove(const char* c_pData);
	void		PartyStateChange(const char* c_pData);
	void		PartySetMemberLevel(const char* c_pData);
	void		Time(const char * c_pData);
	void		ReloadProto(const char * c_pData);
	void		ChangeName(LPDESC d, const char * data);
	void		AuthLogin(LPDESC d, const char * c_pData);
	void		ItemAward(const char * c_pData);
	void		ChangeEmpirePriv(const char* c_pData);
	void		ChangeGuildPriv(const char* c_pData);
	void		ChangeCharacterPriv(const char* c_pData);
	void		MoneyLog(const char* c_pData);
	void		SetEventFlag(const char* c_pData);
	void		CreateObject(const char * c_pData);
	void		DeleteObject(const char * c_pData);
	void		UpdateLand(const char * c_pData);
	void		Notice(const char * c_pData);
	void		MarriageAdd(TPacketMarriageAdd * p);
	void		MarriageUpdate(TPacketMarriageUpdate * p);
	void		MarriageRemove(TPacketMarriageRemove * p);
	void		WeddingRequest(TPacketWeddingRequest* p);
	void		WeddingReady(TPacketWeddingReady* p);
	void		WeddingStart(TPacketWeddingStart* p);
	void		WeddingEnd(TPacketWeddingEnd* p);
	void		TakeMonarchMoney(LPDESC d, const char * data );
	void		AddMonarchMoney(LPDESC d, const char * data );
	void		DecMonarchMoney(LPDESC d, const char * data );
	void		SetMonarch( LPDESC d, const char * data );
	void		ChangeMonarchLord(TPacketChangeMonarchLordACK* data);
	void		UpdateMonarchInfo(TMonarchInfo* data);
	void		AddBlockCountryIp(TPacketBlockCountryIp * data);
	void		BlockException(TPacketBlockException * data);
	void		MyshopPricelistRes( LPDESC d, const TPacketMyshopPricelistHeader* p );
	void 		ReloadAdmin( const char * c_pData );
	void		PrivateShop(LPDESC d, const char* c_pData);
	void		DetailLog(const TPacketNeedLoginLogInfo* info);
	void		ItemAwardInformer(TPacketItemAwardInfromer* data);
	void		ReadOfflineMessages(LPDESC desc, const char* pcData);
	void		RespondChannelStatus(LPDESC desc, const char* pcData);
	void		EventNotification(const char* c_pData);
	void		MoveChannel(LPDESC desc, const char* pcData);
	void 		TempWorldBossRanking(const LPDESC c_lpDesc, const char* c_pszData);
	void		GuildDungeon(const char* c_pData);
	void		GuildDungeonCD(const char* c_pData);
	protected:
		DWORD		m_dwHandle;
};
class CInputUDP : public CInputProcessor
{
	public:
		CInputUDP();
		virtual bool Process(LPDESC d, const void * c_pvOrig, int iBytes, int & r_iBytesProceed);

		virtual BYTE GetType() { return INPROC_UDP; }
		void		SetSockAddr(struct sockaddr_in & rSockAddr) { m_SockAddr = rSockAddr; };
	protected:
		virtual int	Analyze(LPDESC d, BYTE bHeader, const char * c_pData);
	protected:
		void		Handshake(LPDESC lpDesc, const char * c_pData);
		void		StateChecker(const char * c_pData);
	protected:
		struct sockaddr_in	m_SockAddr;
		CPacketInfoUDP 		m_packetInfoUDP;
};
class CInputP2P : public CInputProcessor
{
	public:
		CInputP2P();
		virtual BYTE	GetType() { return INPROC_P2P; }
	protected:
		virtual int	Analyze(LPDESC d, BYTE bHeader, const char * c_pData);
	public:
		void		Setup(LPDESC d, const char * c_pData);
		void		Login(LPDESC d, const char * c_pData);
		void		Logout(LPDESC d, const char * c_pData);
		int			Relay(LPDESC d, const char * c_pData, size_t uiBytes);
#ifdef ENABLE_FULL_NOTICE
		int			Notice(LPDESC d, const char * c_pData, size_t uiBytes, bool bBigFont=false);
#else
		int			Notice(LPDESC d, const char * c_pData, size_t uiBytes);
#endif
		int			MonarchNotice(LPDESC d, const char * c_pData, size_t uiBytes);
		int			MonarchTransfer(LPDESC d, const char * c_pData);
		int			Guild(LPDESC d, const char* c_pData, size_t uiBytes);
		void		Shout(const char * c_pData);
		void		Disconnect(const char * c_pData);
		void		MessengerAdd(const char * c_pData);
		void		MessengerRemove(const char * c_pData);
		void		FindPosition(LPDESC d, const char* c_pData);
		void		WarpCharacter(const char* c_pData);
		void		GuildWarZoneMapIndex(const char* c_pData);
		void		Transfer(const char * c_pData);
		void		XmasWarpSanta(const char * c_pData);
		void		XmasWarpSantaReply(const char * c_pData);
		void		LoginPing(LPDESC d, const char * c_pData);
		void		BlockChat(const char * c_pData);
		void		PCBangUpdate(const char* c_pData);
		void		IamAwake(LPDESC d, const char * c_pData);
		void		Event(const char* c_pData);
		void		HideAndSeekEvent(const char* c_pData);
		void		Switchbot(LPDESC d, const char* c_pData);
		void		LocaleNotice(const char* c_pData);
		void 		MultiFarm(const char* c_pData);
		void		PrivateShopItemSearchUpdate(const char* c_pData);
		void		PrivateShopItemSearch(const char* c_pData);
		int			PrivateShopItemSearchResult(const char* c_pData, size_t uiBytes);
	protected:
		CPacketInfoGG 	m_packetInfoGG;
};
class CInputAuth : public CInputProcessor
{
	public:
		CInputAuth();
		virtual BYTE GetType() { return INPROC_AUTH; }
	protected:
		virtual int	Analyze(LPDESC d, BYTE bHeader, const char * c_pData);
	public:
		void		Login(LPDESC d, const char * c_pData);

};
#endif

