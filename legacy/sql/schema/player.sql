-- MySQL dump 10.13  Distrib 5.6.51, for Linux (x86_64)
--
-- Host: localhost    Database: player
-- ------------------------------------------------------
-- Server version	5.6.51

/*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */;
/*!40101 SET @OLD_CHARACTER_SET_RESULTS=@@CHARACTER_SET_RESULTS */;
/*!40101 SET @OLD_COLLATION_CONNECTION=@@COLLATION_CONNECTION */;
/*!40101 SET NAMES binary */;
/*!40103 SET @OLD_TIME_ZONE=@@TIME_ZONE */;
/*!40103 SET TIME_ZONE='+00:00' */;
/*!40014 SET @OLD_UNIQUE_CHECKS=@@UNIQUE_CHECKS, UNIQUE_CHECKS=0 */;
/*!40014 SET @OLD_FOREIGN_KEY_CHECKS=@@FOREIGN_KEY_CHECKS, FOREIGN_KEY_CHECKS=0 */;
/*!40101 SET @OLD_SQL_MODE=@@SQL_MODE, SQL_MODE='NO_AUTO_VALUE_ON_ZERO' */;
/*!40111 SET @OLD_SQL_NOTES=@@SQL_NOTES, SQL_NOTES=0 */;

--
-- Current Database: `player`
--

CREATE DATABASE /*!32312 IF NOT EXISTS*/ `player` /*!40100 DEFAULT CHARACTER SET latin1 */;

USE `player`;

--
-- Table structure for table `affect`
--

DROP TABLE IF EXISTS `affect`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `affect` (
  `dwPID` int(10) unsigned NOT NULL DEFAULT '0',
  `bType` smallint(5) unsigned NOT NULL DEFAULT '0',
  `bApplyOn` tinyint(4) unsigned NOT NULL DEFAULT '0',
  `lApplyValue` int(11) NOT NULL DEFAULT '0',
  `dwFlag` int(10) unsigned NOT NULL DEFAULT '0',
  `lDuration` int(11) NOT NULL DEFAULT '0',
  `lSPCost` int(11) NOT NULL DEFAULT '0',
  PRIMARY KEY (`dwPID`,`bType`,`bApplyOn`,`lApplyValue`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `banword`
--

DROP TABLE IF EXISTS `banword`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `banword` (
  `word` varchar(24) NOT NULL DEFAULT '',
  PRIMARY KEY (`word`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `change_empire`
--

DROP TABLE IF EXISTS `change_empire`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `change_empire` (
  `change_count` int(11) NOT NULL DEFAULT '0',
  `account_id` int(11) DEFAULT NULL,
  `time` datetime DEFAULT NULL,
  PRIMARY KEY (`change_count`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `daily_gift`
--

DROP TABLE IF EXISTS `daily_gift`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `daily_gift` (
  `player_id` int(11) NOT NULL COMMENT 'Player ID',
  `week` int(11) NOT NULL COMMENT 'Daily Gift Week',
  `day1` int(11) NOT NULL COMMENT 'Day 1 Gift Status',
  `day2` int(11) NOT NULL COMMENT 'Day 2 Gift Status',
  `day3` int(11) NOT NULL COMMENT 'Day 3 Gift Status',
  `day4` int(11) NOT NULL COMMENT 'Day 4 Gift Status',
  `day5` int(11) NOT NULL COMMENT 'Day 5 Gift Status',
  `day6` int(11) NOT NULL COMMENT 'Day 6 Gift Status',
  `day7` int(11) NOT NULL COMMENT 'Day 7 Gift Status',
  `datetime` datetime DEFAULT '0000-00-00 00:00:00' COMMENT 'Week Datetime',
  PRIMARY KEY (`player_id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=utf8 COLLATE=utf8_unicode_ci ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `dungeon_ranking`
--

DROP TABLE IF EXISTS `dungeon_ranking`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `dungeon_ranking` (
  `id` int(10) unsigned NOT NULL AUTO_INCREMENT COMMENT 'Index',
  `pid` int(11) NOT NULL COMMENT 'Player ID',
  `dungeon_index` int(11) NOT NULL COMMENT 'Dungeon Map Index',
  `finish` int(11) NOT NULL COMMENT 'Dungeon Finished',
  `finish_time` int(11) NOT NULL COMMENT 'Dungeon Finished Time',
  `finish_damage` int(11) NOT NULL COMMENT 'Dungeon Finished Damage',
  PRIMARY KEY (`id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `event`
--

DROP TABLE IF EXISTS `event`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `event` (
  `id` int(5) NOT NULL AUTO_INCREMENT,
  `type` enum('EVENT_TYPE_NONE','EVENT_TYPE_EXPERIENCE','EVENT_TYPE_ITEM_DROP','EVENT_TYPE_BOSS','EVENT_TYPE_METIN','EVENT_TYPE_MINING','EVENT_TYPE_GOLD_FROG','EVENT_TYPE_MOONLIGHT','EVENT_TYPE_HEXEGONAL_CHEST','EVENT_TYPE_FISHING','EVENT_TYPE_HIDE_AND_SEEK','EVENT_TYPE_OX','EVENT_TYPE_TANAKA','EVENT_TYPE_SIEGE_WAR','EVENT_TYPE_KINGDOM_WAR') CHARACTER SET latin1 DEFAULT 'EVENT_TYPE_NONE',
  `start` datetime NOT NULL,
  `end` datetime NOT NULL,
  `value0` int(4) DEFAULT '0',
  `value1` int(4) DEFAULT '0',
  `completed` tinyint(1) DEFAULT '0',
  PRIMARY KEY (`id`) USING BTREE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `guild`
--

DROP TABLE IF EXISTS `guild`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `guild` (
  `id` int(10) unsigned NOT NULL AUTO_INCREMENT,
  `name` varchar(12) NOT NULL DEFAULT '' COMMENT 'snprintf(12u)',
  `sp` smallint(6) NOT NULL DEFAULT '1000',
  `master` int(10) unsigned NOT NULL DEFAULT '0',
  `level` tinyint(2) DEFAULT NULL,
  `exp` int(11) DEFAULT NULL,
  `skill_point` tinyint(2) NOT NULL DEFAULT '0',
  `skill` tinyblob,
  `win` int(11) NOT NULL DEFAULT '0',
  `draw` int(11) NOT NULL DEFAULT '0',
  `loss` int(11) NOT NULL DEFAULT '0',
  `ladder_point` int(11) NOT NULL DEFAULT '0',
  `gold` int(11) NOT NULL DEFAULT '0',
  `dungeon_ch` int(11) NOT NULL DEFAULT '0',
  `dungeon_map` int(11) unsigned NOT NULL DEFAULT '0',
  `dungeon_cooldown` int(11) unsigned NOT NULL DEFAULT '0',
  PRIMARY KEY (`id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `guild_comment`
--

DROP TABLE IF EXISTS `guild_comment`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `guild_comment` (
  `id` int(11) unsigned NOT NULL AUTO_INCREMENT,
  `guild_id` int(10) unsigned DEFAULT NULL,
  `name` varchar(24) DEFAULT NULL,
  `notice` tinyint(4) DEFAULT NULL,
  `content` varchar(50) DEFAULT NULL,
  `time` datetime DEFAULT NULL,
  PRIMARY KEY (`id`) USING BTREE,
  KEY `aaa` (`notice`,`id`,`guild_id`) USING BTREE,
  KEY `guild_id` (`guild_id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `guild_grade`
--

DROP TABLE IF EXISTS `guild_grade`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `guild_grade` (
  `guild_id` int(11) unsigned NOT NULL DEFAULT '0',
  `grade` tinyint(4) NOT NULL DEFAULT '0',
  `name` varchar(12) NOT NULL DEFAULT '' COMMENT 'strlen(s) <= 12',
  `auth` set('ADD_MEMBER','REMOVE_MEMEBER','NOTICE','USE_SKILL') DEFAULT NULL,
  PRIMARY KEY (`guild_id`,`grade`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `guild_member`
--

DROP TABLE IF EXISTS `guild_member`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `guild_member` (
  `pid` int(10) unsigned NOT NULL DEFAULT '0',
  `guild_id` int(10) unsigned NOT NULL DEFAULT '0',
  `grade` tinyint(2) DEFAULT NULL,
  `is_general` tinyint(1) NOT NULL DEFAULT '0',
  `offer` int(10) unsigned DEFAULT NULL,
  PRIMARY KEY (`guild_id`,`pid`) USING BTREE,
  UNIQUE KEY `pid` (`pid`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `guild_war`
--

DROP TABLE IF EXISTS `guild_war`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `guild_war` (
  `id_from` int(11) unsigned NOT NULL DEFAULT '0',
  `id_to` int(11) unsigned NOT NULL DEFAULT '0',
  PRIMARY KEY (`id_from`,`id_to`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `guild_war_bet`
--

DROP TABLE IF EXISTS `guild_war_bet`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `guild_war_bet` (
  `login` varchar(16) NOT NULL DEFAULT '',
  `gold` int(10) unsigned NOT NULL DEFAULT '0',
  `guild` int(11) unsigned NOT NULL DEFAULT '0',
  `war_id` int(10) unsigned NOT NULL DEFAULT '0',
  PRIMARY KEY (`war_id`,`login`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `guild_war_reservation`
--

DROP TABLE IF EXISTS `guild_war_reservation`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `guild_war_reservation` (
  `id` int(10) unsigned NOT NULL AUTO_INCREMENT,
  `guild1` int(10) unsigned NOT NULL DEFAULT '0',
  `guild2` int(10) unsigned NOT NULL DEFAULT '0',
  `time` datetime NOT NULL DEFAULT CURRENT_TIMESTAMP,
  `type` tinyint(2) unsigned NOT NULL DEFAULT '0',
  `warprice` int(10) unsigned NOT NULL DEFAULT '0',
  `initscore` int(10) unsigned NOT NULL DEFAULT '0',
  `started` tinyint(1) NOT NULL DEFAULT '0',
  `bet_from` int(10) unsigned NOT NULL DEFAULT '0',
  `bet_to` int(10) unsigned NOT NULL DEFAULT '0',
  `winner` int(11) NOT NULL DEFAULT '-1',
  `power1` int(11) NOT NULL DEFAULT '0',
  `power2` int(11) NOT NULL DEFAULT '0',
  `handicap` int(11) NOT NULL DEFAULT '0',
  `result1` int(11) NOT NULL DEFAULT '0',
  `result2` int(11) NOT NULL DEFAULT '0',
  PRIMARY KEY (`id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `highscore`
--

DROP TABLE IF EXISTS `highscore`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `highscore` (
  `pid` int(11) DEFAULT NULL,
  `board` varchar(0) DEFAULT NULL,
  `value` varchar(0) DEFAULT NULL
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `horse_name`
--

DROP TABLE IF EXISTS `horse_name`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `horse_name` (
  `id` int(11) unsigned NOT NULL DEFAULT '0',
  `name` varchar(16) NOT NULL DEFAULT 'NONAME' COMMENT 'CHARACTER_NAME_MAX_LEN+1 so 24+null',
  PRIMARY KEY (`id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `ishop_data`
--

DROP TABLE IF EXISTS `ishop_data`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `ishop_data` (
  `id` int(11) NOT NULL DEFAULT '0',
  `categoryType` int(11) NOT NULL DEFAULT '0',
  `categorySubType` int(11) NOT NULL DEFAULT '0',
  `itemVnum` int(11) DEFAULT '0',
  `itemPrice` bigint(55) DEFAULT '0',
  `discount` int(11) DEFAULT '0',
  `offerTime` datetime DEFAULT '0000-00-00 00:00:00',
  `addedTime` datetime DEFAULT '0000-00-00 00:00:00',
  `sellCount` bigint(55) DEFAULT '0',
  `week_limit` int(11) DEFAULT '0',
  `month_limit` int(11) DEFAULT '0',
  `maxSellCount` int(11) DEFAULT '-1',
  PRIMARY KEY (`id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `ishop_log`
--

DROP TABLE IF EXISTS `ishop_log`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `ishop_log` (
  `accountID` int(11) NOT NULL DEFAULT '0',
  `playerName` varchar(50) DEFAULT '',
  `buyDate` datetime DEFAULT '0000-00-00 00:00:00' ON UPDATE CURRENT_TIMESTAMP,
  `buyTime` int(11) DEFAULT '0',
  `ipAdress` varchar(16) DEFAULT NULL,
  `itemID` int(11) DEFAULT '0',
  `itemVnum` int(11) DEFAULT '0',
  `itemCount` int(11) DEFAULT '0',
  `itemPrice` bigint(55) DEFAULT '0'
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `item`
--

DROP TABLE IF EXISTS `item`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `item` (
  `id` int(11) unsigned NOT NULL AUTO_INCREMENT,
  `owner_id` int(11) unsigned NOT NULL DEFAULT '0',
  `window` enum('INVENTORY','EQUIPMENT','SAFEBOX','MALL','DRAGON_SOUL_INVENTORY','ATTR67_ADD','SWITCHBOT','AURA_REFINE','BELT_INVENTORY','GROUND') NOT NULL DEFAULT 'INVENTORY',
  `pos` smallint(5) unsigned NOT NULL DEFAULT '0',
  `count` smallint(4) unsigned NOT NULL DEFAULT '0',
  `vnum` int(11) unsigned NOT NULL DEFAULT '0',
  `refine_element` int(11) unsigned NOT NULL DEFAULT '0',
  `transmutation` int(11) unsigned NOT NULL DEFAULT '0',
  `socket0` int(10) unsigned NOT NULL DEFAULT '0',
  `socket1` int(10) unsigned NOT NULL DEFAULT '0',
  `socket2` int(10) unsigned NOT NULL DEFAULT '0',
  `socket3` int(10) unsigned NOT NULL DEFAULT '0',
  `socket4` int(10) unsigned NOT NULL DEFAULT '0',
  `socket5` int(10) unsigned NOT NULL DEFAULT '0',
  `attrtype0` tinyint(4) NOT NULL DEFAULT '0',
  `apply_path3` tinyint(4) NOT NULL DEFAULT '0',
  `apply_value3` smallint(6) NOT NULL DEFAULT '0',
  `apply_type3` tinyint(4) NOT NULL DEFAULT '0',
  `apply_path2` tinyint(4) NOT NULL DEFAULT '0',
  `apply_value2` smallint(6) NOT NULL DEFAULT '0',
  `apply_type2` tinyint(4) NOT NULL DEFAULT '0',
  `apply_path1` tinyint(4) NOT NULL DEFAULT '0',
  `apply_value1` smallint(6) NOT NULL DEFAULT '0',
  `apply_type1` tinyint(4) NOT NULL DEFAULT '0',
  `apply_path0` tinyint(4) NOT NULL DEFAULT '0',
  `apply_value0` smallint(6) NOT NULL DEFAULT '0',
  `apply_type0` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue0` smallint(6) NOT NULL DEFAULT '0',
  `attrtype1` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue1` smallint(6) NOT NULL DEFAULT '0',
  `attrtype2` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue2` smallint(6) NOT NULL DEFAULT '0',
  `attrtype3` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue3` smallint(6) NOT NULL DEFAULT '0',
  `attrtype4` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue4` smallint(6) NOT NULL DEFAULT '0',
  `attrtype5` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue5` smallint(6) NOT NULL DEFAULT '0',
  `attrtype6` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue6` smallint(6) NOT NULL DEFAULT '0',
  PRIMARY KEY (`id`) USING BTREE,
  KEY `owner_id_idx` (`owner_id`) USING BTREE,
  KEY `item_vnum_index` (`vnum`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `item_attr`
--

DROP TABLE IF EXISTS `item_attr`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `item_attr` (
  `apply` enum('MAX_HP','MAX_SP','CON','INT','STR','DEX','ATT_SPEED','MOV_SPEED','CAST_SPEED','HP_REGEN','SP_REGEN','POISON_PCT','STUN_PCT','SLOW_PCT','CRITICAL_PCT','PENETRATE_PCT','ATTBONUS_HUMAN','ATTBONUS_ANIMAL','ATTBONUS_ORC','ATTBONUS_MILGYO','ATTBONUS_UNDEAD','ATTBONUS_DEVIL','STEAL_HP','STEAL_SP','MANA_BURN_PCT','DAMAGE_SP_RECOVER','BLOCK','DODGE','RESIST_SWORD','RESIST_TWOHAND','RESIST_DAGGER','RESIST_BELL','RESIST_FAN','RESIST_BOW','RESIST_FIRE','RESIST_ELEC','RESIST_MAGIC','RESIST_WIND','REFLECT_MELEE','REFLECT_CURSE','POISON_REDUCE','KILL_SP_RECOVER','EXP_DOUBLE_BONUS','GOLD_DOUBLE_BONUS','ITEM_DROP_BONUS','POTION_BONUS','KILL_HP_RECOVER','IMMUNE_STUN','IMMUNE_SLOW','IMMUNE_FALL','SKILL','BOW_DISTANCE','ATT_GRADE_BONUS','DEF_GRADE_BONUS','MAGIC_ATT_GRADE','MAGIC_DEF_GRADE','CURSE_PCT','MAX_STAMINA','ATT_BONUS_TO_WARRIOR','ATT_BONUS_TO_ASSASSIN','ATT_BONUS_TO_SURA','ATT_BONUS_TO_SHAMAN','NORMAL_HIT_DEFEND_BONUS','SKILL_DEFEND_BONUS','NOUSE1','NOUSE2''NOUSE3','NOUSE4','NOUSE5','NOUSE6','NOUSE7','NOUSE8','NOUSE9','NOUSE10','NOUSE11','NOUSE12','NOUSE13','NOUSE14','RESIST_WARRIOR','RESIST_ASSASSIN','RESIST_SURA','RESIST_SHAMAN') NOT NULL DEFAULT 'MAX_HP',
  `prob` varchar(100) NOT NULL DEFAULT '',
  `lv1` varchar(100) NOT NULL DEFAULT '',
  `lv2` varchar(100) NOT NULL DEFAULT '',
  `lv3` varchar(100) NOT NULL DEFAULT '',
  `lv4` varchar(100) NOT NULL DEFAULT '',
  `lv5` varchar(100) NOT NULL DEFAULT '',
  `weapon` varchar(100) NOT NULL DEFAULT '',
  `body` varchar(100) NOT NULL DEFAULT '',
  `wrist` varchar(100) NOT NULL DEFAULT '',
  `foots` varchar(100) NOT NULL DEFAULT '',
  `neck` varchar(100) NOT NULL DEFAULT '',
  `head` varchar(100) NOT NULL DEFAULT '',
  `shield` varchar(100) NOT NULL DEFAULT '',
  `ear` varchar(100) NOT NULL DEFAULT '',
  `talisman` varchar(100) NOT NULL DEFAULT '',
  `glove` varchar(100) NOT NULL DEFAULT ''
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `item_attr_rare`
--

DROP TABLE IF EXISTS `item_attr_rare`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `item_attr_rare` (
  `apply` enum('MAX_HP','MAX_SP','CON','INT','STR','DEX','ATT_SPEED','MOV_SPEED','CAST_SPEED','HP_REGEN','SP_REGEN','POISON_PCT','STUN_PCT','SLOW_PCT','CRITICAL_PCT','PENETRATE_PCT','ATTBONUS_HUMAN','ATTBONUS_ANIMAL','ATTBONUS_ORC','ATTBONUS_MILGYO','ATTBONUS_UNDEAD','ATTBONUS_DEVIL','STEAL_HP','STEAL_SP','MANA_BURN_PCT','DAMAGE_SP_RECOVER','BLOCK','DODGE','RESIST_SWORD','RESIST_TWOHAND','RESIST_DAGGER','RESIST_BELL','RESIST_FAN','RESIST_BOW','RESIST_FIRE','RESIST_ELEC','RESIST_MAGIC','RESIST_WIND','REFLECT_MELEE','REFLECT_CURSE','POISON_REDUCE','KILL_SP_RECOVER','EXP_DOUBLE_BONUS','GOLD_DOUBLE_BONUS','ITEM_DROP_BONUS','POTION_BONUS','KILL_HP_RECOVER','IMMUNE_STUN','IMMUNE_SLOW','IMMUNE_FALL','SKILL','BOW_DISTANCE','ATT_GRADE_BONUS','DEF_GRADE_BONUS','MAGIC_ATT_GRADE_BONUS','MAGIC_DEF_GRADE_BONUS','CURSE_PCT','MAX_STAMINA','ATT_BONUS_TO_WARRIOR','ATT_BONUS_TO_ASSASSIN','ATT_BONUS_TO_SURA','ATT_BONUS_TO_SHAMAN','ATT_BONUS_TO_MONSTER','ATT_BONUS','MALL_DEFBONUS','MALL_EXPBONUS','MALL_ITEMBONUS','MALL_GOLDBONUS','MAX_HP_PCT','MAX_SP_PCT','SKILL_DAMAGE_BONUS','NORMAL_HIT_DAMAGE_BONUS','SKILL_DEFEND_BONUS','NORMAL_HIT_DEFEND_BONUS','EXTRACT_HP_PCT','RESIST_WARRIOR','RESIST_ASSASSIN','RESIST_SURA','RESIST_SHAMAN','ENERGY','DEF_GRADE','COSTUME_ATTR_BONUS','MAGIC_ATT_BONUS_PER','MELEE_MAGIC_ATT_BONUS_PER','RESIST_ICE','RESIST_EARTH','RESIST_DARK','RESIST_CRITICAL','RESIST_PENETRATE','ATTBONUS_METIN','ATTBONUS_BOSS') NOT NULL DEFAULT 'MAX_HP',
  `prob` int(11) unsigned NOT NULL DEFAULT '0',
  `lv1` int(11) unsigned NOT NULL DEFAULT '0',
  `lv2` int(11) unsigned NOT NULL DEFAULT '0',
  `lv3` int(11) unsigned NOT NULL DEFAULT '0',
  `lv4` int(11) unsigned NOT NULL DEFAULT '0',
  `lv5` int(11) unsigned NOT NULL DEFAULT '0',
  `weapon` int(11) unsigned NOT NULL DEFAULT '0',
  `body` int(11) unsigned NOT NULL DEFAULT '0',
  `wrist` int(11) unsigned NOT NULL DEFAULT '0',
  `foots` int(11) unsigned NOT NULL DEFAULT '0',
  `neck` int(11) unsigned NOT NULL DEFAULT '0',
  `head` int(11) unsigned NOT NULL DEFAULT '0',
  `shield` int(11) unsigned NOT NULL DEFAULT '0',
  `ear` int(11) unsigned NOT NULL DEFAULT '0',
  `costume_body` int(11) unsigned NOT NULL DEFAULT '0',
  `costume_hair` int(11) unsigned NOT NULL DEFAULT '0',
  `costume_weapon` int(11) unsigned NOT NULL DEFAULT '0'
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `item_award`
--

DROP TABLE IF EXISTS `item_award`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `item_award` (
  `id` int(11) unsigned NOT NULL AUTO_INCREMENT,
  `pid` int(10) unsigned NOT NULL DEFAULT '0',
  `login` varchar(16) NOT NULL DEFAULT '' COMMENT 'LOGIN_MAX_LEN=30',
  `vnum` int(6) unsigned NOT NULL DEFAULT '0',
  `count` int(10) unsigned NOT NULL DEFAULT '0',
  `given_time` datetime NOT NULL DEFAULT CURRENT_TIMESTAMP,
  `taken_time` datetime DEFAULT NULL,
  `item_id` int(11) unsigned DEFAULT NULL,
  `why` varchar(128) DEFAULT NULL,
  `socket0` int(11) unsigned NOT NULL DEFAULT '0',
  `socket1` int(11) unsigned NOT NULL DEFAULT '0',
  `socket2` int(11) unsigned NOT NULL DEFAULT '0',
  `mall` tinyint(1) NOT NULL DEFAULT '0',
  PRIMARY KEY (`id`) USING BTREE,
  KEY `pid_idx` (`pid`) USING BTREE,
  KEY `given_time_idx` (`given_time`) USING BTREE,
  KEY `taken_time_idx` (`taken_time`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `item_proto`
--

DROP TABLE IF EXISTS `item_proto`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `item_proto` (
  `vnum` int(11) unsigned NOT NULL DEFAULT '0',
  `name` varbinary(24) NOT NULL DEFAULT 'Noname',
  `locale_name` varbinary(24) NOT NULL DEFAULT 'Noname',
  `type` tinyint(2) NOT NULL DEFAULT '0',
  `subtype` tinyint(2) NOT NULL DEFAULT '0',
  `weight` tinyint(3) DEFAULT '0',
  `size` tinyint(3) DEFAULT '0',
  `antiflag` int(11) unsigned DEFAULT '0',
  `flag` int(11) unsigned DEFAULT '0',
  `wearflag` int(11) unsigned DEFAULT '0',
  `immuneflag` set('PARA','CURSE','STUN','SLEEP','SLOW','POISON','TERROR') NOT NULL DEFAULT '',
  `gold` bigint(255) DEFAULT '0',
  `shop_buy_price` int(10) unsigned NOT NULL DEFAULT '0',
  `refined_vnum` int(10) unsigned NOT NULL DEFAULT '0',
  `refine_set` smallint(11) unsigned NOT NULL DEFAULT '0',
  `refine_set2` smallint(5) unsigned NOT NULL DEFAULT '0',
  `magic_pct` tinyint(4) NOT NULL DEFAULT '0',
  `limittype0` tinyint(4) DEFAULT '0',
  `limitvalue0` int(11) DEFAULT '0',
  `limittype1` tinyint(4) DEFAULT '0',
  `limitvalue1` int(11) DEFAULT '0',
  `applytype0` tinyint(4) DEFAULT '0',
  `applyvalue0` int(11) DEFAULT '0',
  `applytype1` tinyint(4) DEFAULT '0',
  `applyvalue1` int(11) DEFAULT '0',
  `applytype2` tinyint(4) DEFAULT '0',
  `applyvalue2` int(11) DEFAULT '0',
  `value0` int(11) DEFAULT '0',
  `value1` int(11) DEFAULT '0',
  `value2` int(11) DEFAULT '0',
  `value3` int(11) DEFAULT '0',
  `value4` int(11) DEFAULT '0',
  `value5` int(11) DEFAULT '0',
  `socket0` tinyint(4) DEFAULT '-1',
  `socket1` tinyint(4) DEFAULT '-1',
  `socket2` tinyint(4) DEFAULT '-1',
  `socket3` tinyint(4) DEFAULT '-1',
  `socket4` tinyint(4) DEFAULT '-1',
  `socket5` tinyint(4) DEFAULT '-1',
  `specular` tinyint(4) NOT NULL DEFAULT '0',
  `socket_pct` tinyint(4) NOT NULL DEFAULT '0',
  `addon_type` smallint(6) NOT NULL DEFAULT '0',
  PRIMARY KEY (`vnum`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `land`
--

DROP TABLE IF EXISTS `land`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `land` (
  `id` int(11) unsigned NOT NULL AUTO_INCREMENT,
  `map_index` int(11) unsigned NOT NULL DEFAULT '0',
  `x` int(11) unsigned NOT NULL DEFAULT '0',
  `y` int(11) unsigned NOT NULL DEFAULT '0',
  `width` int(11) unsigned NOT NULL DEFAULT '0',
  `height` int(11) unsigned NOT NULL DEFAULT '0',
  `guild_id` int(10) unsigned NOT NULL DEFAULT '0',
  `guild_level_limit` tinyint(4) unsigned NOT NULL DEFAULT '0',
  `price` int(10) unsigned NOT NULL DEFAULT '0',
  `enable` enum('YES','NO') NOT NULL DEFAULT 'NO',
  PRIMARY KEY (`id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `lotto_list`
--

DROP TABLE IF EXISTS `lotto_list`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `lotto_list` (
  `id` int(10) unsigned NOT NULL AUTO_INCREMENT,
  `server` varchar(56) DEFAULT NULL COMMENT 'server%s=get_table_postfix(); std::string so dunno; at least 6',
  `pid` int(10) unsigned DEFAULT NULL,
  `time` datetime DEFAULT NULL,
  PRIMARY KEY (`id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `marriage`
--

DROP TABLE IF EXISTS `marriage`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `marriage` (
  `is_married` tinyint(4) NOT NULL DEFAULT '0',
  `pid1` int(10) unsigned NOT NULL DEFAULT '0',
  `pid2` int(10) unsigned NOT NULL DEFAULT '0',
  `love_point` int(11) unsigned DEFAULT NULL,
  `time` int(10) unsigned NOT NULL DEFAULT '0',
  PRIMARY KEY (`pid1`,`pid2`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `messenger_list`
--

DROP TABLE IF EXISTS `messenger_list`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `messenger_list` (
  `account` varchar(16) NOT NULL DEFAULT '' COMMENT '24 at maximum',
  `companion` varchar(16) NOT NULL DEFAULT '' COMMENT '24 at maximum',
  PRIMARY KEY (`account`,`companion`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `mob_proto`
--

DROP TABLE IF EXISTS `mob_proto`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `mob_proto` (
  `vnum` int(11) unsigned NOT NULL DEFAULT '0',
  `name` varchar(24) NOT NULL DEFAULT 'Noname',
  `locale_name` varbinary(24) NOT NULL DEFAULT 'Noname',
  `rank` tinyint(2) unsigned NOT NULL DEFAULT '0',
  `type` tinyint(2) unsigned NOT NULL DEFAULT '0',
  `battle_type` tinyint(1) unsigned NOT NULL DEFAULT '0',
  `level` smallint(3) unsigned NOT NULL DEFAULT '1',
  `size` enum('SMALL','MEDIUM','BIG') NOT NULL DEFAULT 'SMALL',
  `ai_flag` set('AGGR','NOMOVE','COWARD','NOATTSHINSU','NOATTCHUNJO','NOATTJINNO','ATTMOB','BERSERK','STONESKIN','GODSPEED','DEATHBLOW','REVIVE') DEFAULT NULL,
  `mount_capacity` tinyint(2) unsigned NOT NULL DEFAULT '0',
  `setRaceFlag` set('ANIMAL','UNDEAD','DEVIL','HUMAN','ORC','MILGYO','INSECT','FIRE','ICE','DESERT','TREE','ATT_ELEC','ATT_FIRE','ATT_ICE','ATT_WIND','ATT_EARTH','ATT_DARK','METIN','BOSS') NOT NULL DEFAULT '',
  `setImmuneFlag` set('STUN','SLOW','FALL','CURSE','POISON','TERROR','REFLECT') NOT NULL DEFAULT '',
  `empire` tinyint(4) unsigned NOT NULL DEFAULT '0',
  `folder` varchar(100) NOT NULL DEFAULT '',
  `on_click` tinyint(4) unsigned NOT NULL DEFAULT '0',
  `st` smallint(5) unsigned NOT NULL DEFAULT '0',
  `dx` smallint(5) unsigned NOT NULL DEFAULT '0',
  `ht` smallint(5) unsigned NOT NULL DEFAULT '0',
  `iq` smallint(5) unsigned NOT NULL DEFAULT '0',
  `damage_min` smallint(5) unsigned NOT NULL DEFAULT '0',
  `damage_max` smallint(5) unsigned NOT NULL DEFAULT '0',
  `max_hp` int(10) unsigned NOT NULL DEFAULT '0',
  `regen_cycle` tinyint(3) unsigned NOT NULL DEFAULT '0',
  `regen_percent` tinyint(3) unsigned NOT NULL DEFAULT '0',
  `gold_min` int(11) NOT NULL DEFAULT '0',
  `gold_max` int(11) NOT NULL DEFAULT '0',
  `exp` int(10) unsigned NOT NULL DEFAULT '0',
  `def` smallint(5) unsigned NOT NULL DEFAULT '0',
  `attack_speed` smallint(6) unsigned NOT NULL DEFAULT '100',
  `move_speed` smallint(6) unsigned NOT NULL DEFAULT '100',
  `aggressive_hp_pct` tinyint(3) unsigned NOT NULL DEFAULT '0',
  `aggressive_sight` smallint(10) unsigned NOT NULL DEFAULT '0',
  `attack_range` smallint(5) unsigned NOT NULL DEFAULT '0',
  `drop_item` int(10) unsigned NOT NULL DEFAULT '0',
  `resurrection_vnum` int(10) unsigned NOT NULL DEFAULT '0',
  `enchant_curse` tinyint(4) unsigned NOT NULL DEFAULT '0',
  `enchant_slow` tinyint(4) unsigned NOT NULL DEFAULT '0',
  `enchant_poison` tinyint(4) unsigned NOT NULL DEFAULT '0',
  `enchant_stun` tinyint(3) unsigned NOT NULL DEFAULT '0',
  `enchant_critical` tinyint(3) unsigned NOT NULL DEFAULT '0',
  `enchant_penetrate` tinyint(3) unsigned NOT NULL DEFAULT '0',
  `resist_sword` tinyint(4) NOT NULL DEFAULT '0',
  `resist_twohand` tinyint(4) NOT NULL DEFAULT '0',
  `resist_dagger` tinyint(4) NOT NULL DEFAULT '0',
  `resist_bell` tinyint(4) NOT NULL DEFAULT '0',
  `resist_fan` tinyint(4) NOT NULL DEFAULT '0',
  `resist_bow` tinyint(4) NOT NULL DEFAULT '0',
  `resist_fire` tinyint(4) NOT NULL DEFAULT '0',
  `resist_elect` tinyint(4) NOT NULL DEFAULT '0',
  `resist_magic` tinyint(4) NOT NULL DEFAULT '0',
  `resist_wind` tinyint(4) NOT NULL DEFAULT '0',
  `resist_poison` tinyint(4) NOT NULL DEFAULT '0',
  `dam_multiply` float DEFAULT NULL,
  `summon` int(11) DEFAULT NULL,
  `drain_sp` int(11) DEFAULT NULL,
  `mob_color` int(10) unsigned DEFAULT NULL,
  `polymorph_item` int(10) unsigned NOT NULL DEFAULT '0',
  `skill_level0` tinyint(3) unsigned DEFAULT NULL,
  `skill_vnum0` int(10) unsigned DEFAULT NULL,
  `skill_level1` tinyint(3) unsigned DEFAULT NULL,
  `skill_vnum1` int(10) unsigned DEFAULT NULL,
  `skill_level2` tinyint(3) unsigned DEFAULT NULL,
  `skill_vnum2` int(10) unsigned DEFAULT NULL,
  `skill_level3` tinyint(3) unsigned DEFAULT NULL,
  `skill_vnum3` int(10) unsigned DEFAULT NULL,
  `skill_level4` tinyint(3) unsigned DEFAULT NULL,
  `skill_vnum4` int(10) unsigned DEFAULT NULL,
  `sp_berserk` tinyint(4) NOT NULL DEFAULT '0',
  `sp_stoneskin` tinyint(4) NOT NULL DEFAULT '0',
  `sp_godspeed` tinyint(4) NOT NULL DEFAULT '0',
  `sp_deathblow` tinyint(4) NOT NULL DEFAULT '0',
  `sp_revive` tinyint(4) NOT NULL DEFAULT '0',
  PRIMARY KEY (`vnum`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `monarch`
--

DROP TABLE IF EXISTS `monarch`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `monarch` (
  `empire` int(10) unsigned NOT NULL DEFAULT '0',
  `pid` int(10) unsigned DEFAULT NULL,
  `windate` datetime DEFAULT NULL,
  `money` bigint(20) unsigned DEFAULT NULL,
  PRIMARY KEY (`empire`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `monarch_candidacy`
--

DROP TABLE IF EXISTS `monarch_candidacy`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `monarch_candidacy` (
  `pid` int(10) unsigned NOT NULL DEFAULT '0',
  `date` datetime DEFAULT CURRENT_TIMESTAMP,
  `name` varchar(16) DEFAULT NULL,
  `windate` datetime DEFAULT NULL,
  PRIMARY KEY (`pid`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `monarch_election`
--

DROP TABLE IF EXISTS `monarch_election`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `monarch_election` (
  `pid` int(10) unsigned NOT NULL DEFAULT '0',
  `selectedpid` int(10) unsigned DEFAULT '0',
  `electiondata` datetime DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (`pid`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `myshop_pricelist`
--

DROP TABLE IF EXISTS `myshop_pricelist`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `myshop_pricelist` (
  `owner_id` int(11) unsigned NOT NULL DEFAULT '0',
  `item_vnum` int(11) unsigned NOT NULL DEFAULT '0',
  `price` int(10) unsigned NOT NULL DEFAULT '0',
  UNIQUE KEY `list_id` (`owner_id`,`item_vnum`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `object`
--

DROP TABLE IF EXISTS `object`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `object` (
  `id` int(11) unsigned NOT NULL AUTO_INCREMENT,
  `land_id` int(11) unsigned NOT NULL DEFAULT '0',
  `vnum` int(10) unsigned NOT NULL DEFAULT '0',
  `map_index` int(11) unsigned NOT NULL DEFAULT '0',
  `x` int(11) NOT NULL DEFAULT '0',
  `y` int(11) NOT NULL DEFAULT '0',
  `x_rot` float NOT NULL DEFAULT '0',
  `y_rot` float NOT NULL DEFAULT '0',
  `z_rot` float NOT NULL DEFAULT '0',
  `life` int(11) NOT NULL DEFAULT '0',
  PRIMARY KEY (`id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `object_proto`
--

DROP TABLE IF EXISTS `object_proto`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `object_proto` (
  `vnum` int(10) unsigned NOT NULL DEFAULT '0',
  `name` varchar(32) NOT NULL DEFAULT '',
  `price` int(10) unsigned NOT NULL DEFAULT '0',
  `materials` varchar(64) NOT NULL DEFAULT '',
  `upgrade_vnum` int(10) unsigned NOT NULL DEFAULT '0',
  `upgrade_limit_time` int(10) unsigned NOT NULL DEFAULT '0',
  `life` int(11) NOT NULL DEFAULT '0',
  `reg_1` int(11) NOT NULL DEFAULT '0',
  `reg_2` int(11) NOT NULL DEFAULT '0',
  `reg_3` int(11) NOT NULL DEFAULT '0',
  `reg_4` int(11) NOT NULL DEFAULT '0',
  `npc` int(10) unsigned NOT NULL DEFAULT '0',
  `group_vnum` int(10) unsigned NOT NULL DEFAULT '0',
  `dependent_group` int(10) unsigned NOT NULL DEFAULT '0',
  PRIMARY KEY (`vnum`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `player`
--

DROP TABLE IF EXISTS `player`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `player` (
  `id` int(11) unsigned NOT NULL AUTO_INCREMENT,
  `account_id` int(11) unsigned NOT NULL DEFAULT '0',
  `name` varchar(24) NOT NULL DEFAULT 'NONAME',
  `job` tinyint(2) unsigned NOT NULL DEFAULT '0',
  `voice` tinyint(1) NOT NULL DEFAULT '0',
  `dir` tinyint(2) NOT NULL DEFAULT '0',
  `x` int(11) NOT NULL DEFAULT '0',
  `y` int(11) NOT NULL DEFAULT '0',
  `z` int(11) NOT NULL DEFAULT '0',
  `map_index` int(11) NOT NULL DEFAULT '0',
  `exit_x` int(11) NOT NULL DEFAULT '0',
  `exit_y` int(11) NOT NULL DEFAULT '0',
  `exit_map_index` int(11) NOT NULL DEFAULT '0',
  `hp` int(11) NOT NULL DEFAULT '0',
  `mp` int(11) NOT NULL DEFAULT '0',
  `stamina` smallint(6) NOT NULL DEFAULT '0',
  `random_hp` smallint(5) NOT NULL DEFAULT '0' COMMENT 'if lvl 0, it will be negative',
  `random_sp` smallint(5) NOT NULL DEFAULT '0' COMMENT 'if lvl 0, it will be negative',
  `playtime` int(11) NOT NULL DEFAULT '0',
  `level` tinyint(2) unsigned NOT NULL DEFAULT '1',
  `level_step` tinyint(1) NOT NULL DEFAULT '0',
  `private_shop_unlocked_slot` int(11) NOT NULL DEFAULT '0',
  `st` smallint(3) NOT NULL DEFAULT '0',
  `ht` smallint(3) NOT NULL DEFAULT '0',
  `dx` smallint(3) NOT NULL DEFAULT '0',
  `iq` smallint(3) NOT NULL DEFAULT '0',
  `exp` int(11) NOT NULL DEFAULT '0',
  `gold` bigint(255) NOT NULL DEFAULT '0',
  `gaya` int(11) unsigned NOT NULL DEFAULT '0',
  `stat_point` smallint(3) NOT NULL DEFAULT '0',
  `skill_point` smallint(3) NOT NULL DEFAULT '0',
  `quickslot` tinyblob,
  `ip` varchar(15) DEFAULT '0.0.0.0',
  `part_main` mediumint(6) NOT NULL DEFAULT '0',
  `part_base` tinyint(4) NOT NULL DEFAULT '0',
  `part_hair` mediumint(4) NOT NULL DEFAULT '0',
  `part_sash` smallint(4) unsigned NOT NULL DEFAULT '0',
  `skill_group` tinyint(4) NOT NULL DEFAULT '0',
  `skill_level` blob,
  `alignment` int(11) NOT NULL DEFAULT '0',
  `conqueror_level` tinyint(3) DEFAULT '0',
  `conqueror_level_step` tinyint(1) DEFAULT '0',
  `sungma_str` smallint(3) DEFAULT '0',
  `sungma_hp` smallint(3) DEFAULT '0',
  `sungma_move` smallint(3) DEFAULT '0',
  `sungma_immune` smallint(3) DEFAULT '0',
  `conqueror_exp` bigint(255) DEFAULT '0',
  `conqueror_point` smallint(3) DEFAULT '0',
  `last_play` datetime NOT NULL DEFAULT CURRENT_TIMESTAMP,
  `change_name` tinyint(1) NOT NULL DEFAULT '0',
  `sub_skill_point` smallint(3) NOT NULL DEFAULT '0',
  `stat_reset_count` tinyint(4) NOT NULL DEFAULT '0',
  `horse_hp` smallint(4) NOT NULL DEFAULT '0',
  `horse_stamina` smallint(4) NOT NULL DEFAULT '0',
  `horse_level` tinyint(2) unsigned NOT NULL DEFAULT '0',
  `horse_hp_droptime` int(10) unsigned NOT NULL DEFAULT '0',
  `horse_riding` tinyint(1) NOT NULL DEFAULT '0',
  `horse_skill_point` smallint(3) NOT NULL DEFAULT '0',
  `envanter` int(11) NOT NULL DEFAULT '0',
  `fish_slots` blob,
  `battlepass` blob,
  `fish_use_count` datetime DEFAULT NULL,
  `premium` smallint(1) DEFAULT '0',
  `premium_time` bigint(20) DEFAULT '0',
  `secured` smallint(1) DEFAULT '0',
  `secured_password` varchar(6) DEFAULT '999999',
  `biologist_state` smallint(2) DEFAULT '0',
  `biologist_items_taken` smallint(2) DEFAULT '0',
  `biologist_completed` smallint(1) DEFAULT '0',
  PRIMARY KEY (`id`) USING BTREE,
  KEY `account_id_idx` (`account_id`) USING BTREE,
  KEY `name_idx` (`name`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `player_deleted`
--

DROP TABLE IF EXISTS `player_deleted`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `player_deleted` (
  `id` int(11) unsigned NOT NULL,
  `account_id` int(11) unsigned NOT NULL DEFAULT '0',
  `name` varchar(24) NOT NULL DEFAULT 'NONAME',
  `job` tinyint(2) unsigned NOT NULL DEFAULT '0',
  `voice` tinyint(1) NOT NULL DEFAULT '0',
  `dir` tinyint(2) NOT NULL DEFAULT '0',
  `x` int(11) NOT NULL DEFAULT '0',
  `y` int(11) NOT NULL DEFAULT '0',
  `z` int(11) NOT NULL DEFAULT '0',
  `map_index` int(11) NOT NULL DEFAULT '0',
  `exit_x` int(11) NOT NULL DEFAULT '0',
  `exit_y` int(11) NOT NULL DEFAULT '0',
  `exit_map_index` int(11) NOT NULL DEFAULT '0',
  `hp` smallint(4) NOT NULL DEFAULT '0',
  `mp` smallint(4) NOT NULL DEFAULT '0',
  `stamina` smallint(6) NOT NULL DEFAULT '0',
  `random_hp` smallint(5) NOT NULL DEFAULT '0' COMMENT 'if lvl 0, it will be negative',
  `random_sp` smallint(5) NOT NULL DEFAULT '0' COMMENT 'if lvl 0, it will be negative',
  `playtime` int(11) NOT NULL DEFAULT '0',
  `level` tinyint(2) unsigned NOT NULL DEFAULT '1',
  `level_step` tinyint(1) NOT NULL DEFAULT '0',
  `private_shop_unlocked_slot` int(11) NOT NULL DEFAULT '0',
  `st` smallint(3) NOT NULL DEFAULT '0',
  `ht` smallint(3) NOT NULL DEFAULT '0',
  `dx` smallint(3) NOT NULL DEFAULT '0',
  `iq` smallint(3) NOT NULL DEFAULT '0',
  `exp` int(11) NOT NULL DEFAULT '0',
  `gold` int(11) NOT NULL DEFAULT '0',
  `gaya` int(11) unsigned NOT NULL DEFAULT '0',
  `stat_point` smallint(3) NOT NULL DEFAULT '0',
  `skill_point` smallint(3) NOT NULL DEFAULT '0',
  `quickslot` tinyblob,
  `ip` varchar(15) DEFAULT '0.0.0.0',
  `part_main` mediumint(6) NOT NULL DEFAULT '0',
  `part_base` tinyint(4) NOT NULL DEFAULT '0',
  `part_hair` mediumint(4) NOT NULL DEFAULT '0',
  `part_sash` smallint(4) unsigned NOT NULL DEFAULT '0',
  `skill_group` tinyint(4) NOT NULL DEFAULT '0',
  `skill_level` blob,
  `alignment` int(11) NOT NULL DEFAULT '0',
  `last_play` datetime NOT NULL DEFAULT CURRENT_TIMESTAMP,
  `change_name` tinyint(1) NOT NULL DEFAULT '0',
  `sub_skill_point` smallint(3) NOT NULL DEFAULT '0',
  `stat_reset_count` tinyint(4) NOT NULL DEFAULT '0',
  `horse_hp` smallint(4) NOT NULL DEFAULT '0',
  `horse_stamina` smallint(4) NOT NULL DEFAULT '0',
  `horse_level` tinyint(2) unsigned NOT NULL DEFAULT '0',
  `horse_hp_droptime` int(10) unsigned NOT NULL DEFAULT '0',
  `horse_riding` tinyint(1) NOT NULL DEFAULT '0',
  `horse_skill_point` smallint(3) NOT NULL DEFAULT '0',
  `envanter` int(11) NOT NULL DEFAULT '0',
  `fish_slots` blob,
  `fish_use_count` datetime NOT NULL,
  `ptagcolor` varchar(16) CHARACTER SET utf8 COLLATE utf8_unicode_ci DEFAULT NULL,
  `ptag` varchar(16) CHARACTER SET utf8 COLLATE utf8_unicode_ci DEFAULT NULL
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `player_index`
--

DROP TABLE IF EXISTS `player_index`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `player_index` (
  `id` int(11) unsigned NOT NULL DEFAULT '0',
  `pid1` int(11) unsigned NOT NULL DEFAULT '0',
  `pid2` int(11) unsigned NOT NULL DEFAULT '0',
  `pid3` int(11) unsigned NOT NULL DEFAULT '0',
  `pid4` int(11) unsigned NOT NULL DEFAULT '0',
  `pid5` int(10) unsigned NOT NULL DEFAULT '0',
  `empire` tinyint(4) unsigned NOT NULL DEFAULT '0',
  PRIMARY KEY (`id`) USING BTREE,
  KEY `pid1_key` (`pid1`) USING BTREE,
  KEY `pid2_key` (`pid2`) USING BTREE,
  KEY `pid3_key` (`pid3`) USING BTREE,
  KEY `pid4_key` (`pid4`) USING BTREE,
  KEY `pid5_key` (`pid5`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `player_top`
--

DROP TABLE IF EXISTS `player_top`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `player_top` (
  `id` int(11) NOT NULL AUTO_INCREMENT,
  `pid` int(11) DEFAULT NULL,
  `level` int(11) DEFAULT NULL,
  PRIMARY KEY (`id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=utf8 ROW_FORMAT=COMPACT;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `private_shop`
--

DROP TABLE IF EXISTS `private_shop`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `private_shop` (
  `owner_id` bigint(11) NOT NULL,
  `owner_name` varchar(25) NOT NULL COMMENT 'CHARACTER_NAME_MAX_LEN + 1',
  `state` enum('CLOSED','OPEN','MODIFY') NOT NULL DEFAULT 'CLOSED',
  `title` varchar(33) NOT NULL COMMENT 'SHOP_SIGN_MAX_LEN +1',
  `title_type` tinyint(1) NOT NULL,
  `vnum` int(7) NOT NULL,
  `x` int(11) NOT NULL,
  `y` int(11) NOT NULL,
  `map_index` int(3) NOT NULL,
  `channel` tinyint(1) NOT NULL,
  `port` int(5) DEFAULT NULL,
  `gold` bigint(13) NOT NULL,
  `cheque` int(6) NOT NULL,
  `page_count` tinyint(1) NOT NULL DEFAULT '1',
  `premium_time` int(11) NOT NULL DEFAULT '0',
  `unlocked_slots` int(11) NOT NULL DEFAULT '0',
  PRIMARY KEY (`owner_id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `private_shop_item`
--

DROP TABLE IF EXISTS `private_shop_item`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `private_shop_item` (
  `id` bigint(13) unsigned NOT NULL AUTO_INCREMENT,
  `owner_id` bigint(11) unsigned NOT NULL DEFAULT '0',
  `pos` smallint(5) unsigned NOT NULL DEFAULT '0',
  `count` tinyint(3) unsigned NOT NULL DEFAULT '0',
  `vnum` int(11) unsigned NOT NULL DEFAULT '0',
  `gold` bigint(13) unsigned NOT NULL DEFAULT '0',
  `cheque` int(4) unsigned NOT NULL DEFAULT '0',
  `checkin` int(11) unsigned NOT NULL DEFAULT '0',
  `transmutation` int(10) unsigned NOT NULL DEFAULT '0',
  `socket0` int(10) unsigned NOT NULL DEFAULT '0',
  `socket1` int(10) unsigned NOT NULL DEFAULT '0',
  `socket2` int(10) unsigned NOT NULL DEFAULT '0',
  `attrtype0` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue0` smallint(6) NOT NULL DEFAULT '0',
  `attrtype1` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue1` smallint(6) NOT NULL DEFAULT '0',
  `attrtype2` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue2` smallint(6) NOT NULL DEFAULT '0',
  `attrtype3` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue3` smallint(6) NOT NULL DEFAULT '0',
  `attrtype4` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue4` smallint(6) NOT NULL DEFAULT '0',
  `attrtype5` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue5` smallint(6) NOT NULL DEFAULT '0',
  `attrtype6` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue6` smallint(6) NOT NULL DEFAULT '0',
  PRIMARY KEY (`id`) USING BTREE,
  KEY `owner_id_idx` (`owner_id`) USING BTREE,
  KEY `item_vnum_index` (`vnum`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `private_shop_sale_history`
--

DROP TABLE IF EXISTS `private_shop_sale_history`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `private_shop_sale_history` (
  `id` bigint(11) unsigned NOT NULL AUTO_INCREMENT,
  `owner_id` bigint(11) unsigned NOT NULL DEFAULT '0',
  `customer_id` bigint(11) NOT NULL,
  `customer_name` varchar(26) NOT NULL COMMENT 'CHARACTER_NAME_MAX_LEN + 1',
  `time` datetime NOT NULL,
  `item_id` bigint(13) NOT NULL,
  `count` tinyint(3) unsigned NOT NULL DEFAULT '0',
  `vnum` int(11) unsigned NOT NULL DEFAULT '0',
  `gold` bigint(13) unsigned NOT NULL DEFAULT '0',
  `cheque` int(11) unsigned NOT NULL DEFAULT '0',
  `checkin` int(11) unsigned NOT NULL DEFAULT '0',
  `transmutation` int(10) unsigned NOT NULL DEFAULT '0',
  `socket0` int(10) unsigned NOT NULL DEFAULT '0',
  `socket1` int(10) unsigned NOT NULL DEFAULT '0',
  `socket2` int(10) unsigned NOT NULL DEFAULT '0',
  `attrtype0` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue0` smallint(6) NOT NULL DEFAULT '0',
  `attrtype1` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue1` smallint(6) NOT NULL DEFAULT '0',
  `attrtype2` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue2` smallint(6) NOT NULL DEFAULT '0',
  `attrtype3` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue3` smallint(6) NOT NULL DEFAULT '0',
  `attrtype4` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue4` smallint(6) NOT NULL DEFAULT '0',
  `attrtype5` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue5` smallint(6) NOT NULL DEFAULT '0',
  `attrtype6` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue6` smallint(6) NOT NULL DEFAULT '0',
  PRIMARY KEY (`id`) USING BTREE,
  KEY `owner_id_idx` (`owner_id`) USING BTREE,
  KEY `item_vnum_index` (`vnum`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `quest`
--

DROP TABLE IF EXISTS `quest`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `quest` (
  `dwPID` int(10) unsigned NOT NULL DEFAULT '0',
  `szName` varchar(32) NOT NULL DEFAULT '',
  `szState` varchar(64) NOT NULL DEFAULT '',
  `lValue` int(11) NOT NULL DEFAULT '0',
  PRIMARY KEY (`dwPID`,`szName`,`szState`) USING BTREE,
  KEY `pid_idx` (`dwPID`) USING BTREE,
  KEY `name_idx` (`szName`) USING BTREE,
  KEY `state_idx` (`szState`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `rank_global`
--

DROP TABLE IF EXISTS `rank_global`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `rank_global` (
  `pid` int(11) NOT NULL,
  `name` varchar(25) DEFAULT NULL,
  `lv` int(3) DEFAULT NULL,
  `empire` int(3) DEFAULT NULL,
  `type0` int(11) DEFAULT NULL,
  `type1` int(11) DEFAULT NULL,
  `type2` int(11) DEFAULT NULL,
  `type3` int(11) DEFAULT NULL,
  `type4` int(11) DEFAULT NULL,
  `type5` int(11) DEFAULT NULL,
  `type6` int(11) DEFAULT NULL,
  `type7` int(11) DEFAULT NULL,
  `type8` int(11) DEFAULT NULL,
  PRIMARY KEY (`pid`) USING BTREE
) ENGINE=InnoDB DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `rank_global_reward`
--

DROP TABLE IF EXISTS `rank_global_reward`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `rank_global_reward` (
  `type` smallint(3) DEFAULT NULL,
  `pos` int(10) DEFAULT NULL,
  `item_id` int(10) DEFAULT NULL,
  `item_count` int(10) DEFAULT NULL
) ENGINE=InnoDB DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `rank_global_winner`
--

DROP TABLE IF EXISTS `rank_global_winner`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `rank_global_winner` (
  `pid` int(11) DEFAULT NULL,
  `name` varchar(30) DEFAULT NULL,
  `position` int(11) DEFAULT NULL,
  `type` int(3) DEFAULT NULL,
  `vnum` int(4) DEFAULT NULL,
  `count` int(4) DEFAULT NULL,
  `time` datetime DEFAULT NULL,
  `done` int(1) DEFAULT NULL
) ENGINE=InnoDB DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `refine_proto`
--

DROP TABLE IF EXISTS `refine_proto`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `refine_proto` (
  `id` int(11) NOT NULL AUTO_INCREMENT,
  `vnum0` int(10) unsigned NOT NULL DEFAULT '0',
  `count0` smallint(6) NOT NULL DEFAULT '0',
  `vnum1` int(10) unsigned NOT NULL DEFAULT '0',
  `count1` smallint(6) NOT NULL DEFAULT '0',
  `vnum2` int(10) unsigned NOT NULL DEFAULT '0',
  `count2` smallint(6) NOT NULL DEFAULT '0',
  `vnum3` int(10) unsigned NOT NULL DEFAULT '0',
  `count3` smallint(6) NOT NULL DEFAULT '0',
  `vnum4` int(10) unsigned NOT NULL DEFAULT '0',
  `count4` smallint(6) NOT NULL DEFAULT '0',
  `cost` int(11) NOT NULL DEFAULT '0',
  `src_vnum` int(10) unsigned NOT NULL DEFAULT '0',
  `result_vnum` int(10) unsigned NOT NULL DEFAULT '0',
  `prob` smallint(6) NOT NULL DEFAULT '100',
  PRIMARY KEY (`id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `reward_table`
--

DROP TABLE IF EXISTS `reward_table`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `reward_table` (
  `rewardIndex` enum('REWARD_NONE','REWARD_115','REWARD_PET_115','REWARD_120','REWARD_LEGENDARY_SKILL','REWARD_LEGENDARY_SKILL_SET','REWARD_THANDRUIL','REWARD_HYDRA','REWARD_CRYSTAL_DRAGON','REWARD_OFFLINESHOP_SLOT','REWARD_INVENTORY_SLOT','REWARD_AVERAGE','REWARD_ELEMENT','REWARD_BATTLEPASS','REWARD_CUSTOM_SASH','REWARD_AURA','REWARD_ENERGY','REWARD_112_BIO','REWARD_120_BIO','REWARD_LEADER_SHIP','REWARD_BUFFI_LEGENDARY_SKILL') NOT NULL DEFAULT 'REWARD_NONE',
  `lc_text` varchar(255) NOT NULL DEFAULT '',
  `playerName` varchar(255) NOT NULL DEFAULT '',
  `itemVnum0` int(11) NOT NULL DEFAULT '0',
  `itemCount0` int(11) NOT NULL DEFAULT '0',
  `itemVnum1` int(11) NOT NULL DEFAULT '0',
  `itemCount1` int(11) NOT NULL DEFAULT '0',
  `itemVnum2` int(11) NOT NULL DEFAULT '0',
  `itemCount2` int(11) NOT NULL DEFAULT '0'
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `safebox`
--

DROP TABLE IF EXISTS `safebox`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `safebox` (
  `account_id` int(10) unsigned NOT NULL DEFAULT '0',
  `size` tinyint(3) unsigned NOT NULL DEFAULT '0',
  `password` varchar(6) NOT NULL DEFAULT '',
  `gold` int(11) NOT NULL DEFAULT '0',
  PRIMARY KEY (`account_id`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `shop`
--

DROP TABLE IF EXISTS `shop`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `shop` (
  `vnum` int(11) NOT NULL DEFAULT '0',
  `name` varchar(32) NOT NULL DEFAULT 'Noname',
  `npc_vnum` smallint(6) NOT NULL DEFAULT '0',
  PRIMARY KEY (`vnum`)
) ENGINE=MyISAM DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `shop_item`
--

DROP TABLE IF EXISTS `shop_item`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `shop_item` (
  `shop_vnum` int(11) NOT NULL DEFAULT '0',
  `item_vnum` int(11) NOT NULL DEFAULT '0',
  `count` smallint(4) unsigned NOT NULL DEFAULT '1',
  UNIQUE KEY `vnum_unique` (`shop_vnum`,`item_vnum`,`count`)
) ENGINE=MyISAM DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `shopex`
--

DROP TABLE IF EXISTS `shopex`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `shopex` (
  `vnum` int(11) NOT NULL DEFAULT '0',
  `name` varchar(32) NOT NULL DEFAULT 'Noname',
  `npc_vnum` smallint(6) NOT NULL DEFAULT '0',
  PRIMARY KEY (`vnum`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `shopex_item`
--

DROP TABLE IF EXISTS `shopex_item`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `shopex_item` (
  `shop_vnum` int(11) NOT NULL DEFAULT '0',
  `item_vnum` int(11) NOT NULL DEFAULT '0',
  `count` tinyint(3) unsigned NOT NULL DEFAULT '1',
  `price` int(10) unsigned NOT NULL DEFAULT '0',
  `price_vnum` int(10) unsigned NOT NULL DEFAULT '0',
  `price_type` enum('GOLD','SECONDCOIN','ITEM','EXP') NOT NULL DEFAULT 'GOLD',
  `socket0` int(10) unsigned NOT NULL DEFAULT '0',
  `socket1` int(10) unsigned NOT NULL DEFAULT '0',
  `socket2` int(10) unsigned NOT NULL DEFAULT '0',
  `attrtype0` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue0` smallint(6) NOT NULL DEFAULT '0',
  `attrtype1` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue1` smallint(6) NOT NULL DEFAULT '0',
  `attrtype2` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue2` smallint(6) NOT NULL DEFAULT '0',
  `attrtype3` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue3` smallint(6) NOT NULL DEFAULT '0',
  `attrtype4` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue4` smallint(6) NOT NULL DEFAULT '0',
  `attrtype5` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue5` smallint(6) NOT NULL DEFAULT '0',
  `attrtype6` tinyint(4) NOT NULL DEFAULT '0',
  `attrvalue6` smallint(6) NOT NULL DEFAULT '0',
  `socket3` int(10) NOT NULL,
  `socket4` int(10) NOT NULL,
  `socket5` int(10) NOT NULL,
  UNIQUE KEY `vnum_unique` (`shop_vnum`,`item_vnum`,`count`,`price`,`price_vnum`,`price_type`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=FIXED;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `skill_proto`
--

DROP TABLE IF EXISTS `skill_proto`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `skill_proto` (
  `dwVnum` int(11) NOT NULL DEFAULT '0',
  `szName` varchar(32) CHARACTER SET utf8 NOT NULL DEFAULT '',
  `bType` tinyint(4) NOT NULL DEFAULT '0',
  `bLevelStep` tinyint(4) NOT NULL DEFAULT '0',
  `bMaxLevel` tinyint(4) NOT NULL DEFAULT '0',
  `bLevelLimit` tinyint(3) unsigned NOT NULL DEFAULT '0',
  `szPointOn` enum('NONE','MAX_HP','MAX_SP','HP_REGEN','SP_REGEN','BLOCK','HP','SP','ATT_GRADE','DEF_GRADE','MAGIC_ATT_GRADE','MAGIC_DEF_GRADE','BOW_DISTANCE','MOV_SPEED','ATT_SPEED','POISON_PCT','RESIST_RANGE','RESIST_MELEE','CASTING_SPEED','REFLECT_MELEE','ATT_BONUS','DEF_BONUS','RESIST_NORMAL','DODGE','KILL_HP_RECOVER','KILL_SP_RECOVER','HIT_HP_RECOVER','HIT_SP_RECOVER','CRITICAL','MANASHIELD','SKILL_DAMAGE_BONUS','NORMAL_HIT_DAMAGE_BONUS') NOT NULL DEFAULT 'NONE',
  `szPointPoly` varchar(100) NOT NULL DEFAULT '',
  `szSPCostPoly` varchar(100) NOT NULL DEFAULT '',
  `szDurationPoly` varchar(100) NOT NULL DEFAULT '',
  `szDurationSPCostPoly` varchar(100) NOT NULL DEFAULT '',
  `szCooldownPoly` varchar(100) NOT NULL DEFAULT '',
  `szMasterBonusPoly` varchar(100) NOT NULL DEFAULT '',
  `szAttackGradePoly` varchar(100) NOT NULL DEFAULT '',
  `setFlag` set('ATTACK','USE_MELEE_DAMAGE','COMPUTE_ATTGRADE','SELFONLY','USE_MAGIC_DAMAGE','USE_HP_AS_COST','COMPUTE_MAGIC_DAMAGE','SPLASH','GIVE_PENALTY','USE_ARROW_DAMAGE','PENETRATE','IGNORE_TARGET_RATING','ATTACK_SLOW','ATTACK_STUN','HP_ABSORB','SP_ABSORB','ATTACK_FIRE_CONT','REMOVE_BAD_AFFECT','REMOVE_GOOD_AFFECT','CRUSH','ATTACK_POISON','TOGGLE','DISABLE_BY_POINT_UP','CRUSH_LONG','ATTACK_WIND','ATTACK_ELEC','ATTACK_FIRE','ATTACK_BLEEDING','PARTY') DEFAULT NULL,
  `setAffectFlag` enum('YMIR','INVISIBILITY','SPAWN','POISON','SLOW','STUN','DUNGEON_READY','DUNGEON_UNIQUE','BUILDING_CONSTRUCTION_SMALL','BUILDING_CONSTRUCTION_LARGE','BUILDING_UPGRADE','MOV_SPEED_POTION','ATT_SPEED_POTION','FISH_MIND','JEONGWIHON','GEOMGYEONG','CHEONGEUN','GYEONGGONG','EUNHYUNG','GWIGUM','TERROR','JUMAGAP','HOSIN','BOHO','KWAESOK','MANASHIELD','MUYEONG','REVIVE_INVISIBLE','FIRE','GICHEON','JEUNGRYEOK','TANHWAN_DASH','PABEOP','CHEONGEUN_WITH_FALL','POLYMORPH','WAR_FLAG1','WAR_FLAG2','WAR_FLAG3','CHINA_FIREWORK','HAIR','GERMANY','RAMADAN_RING','BLEEDING','RED_POSSESSION','BLUE_POSSESSION') NOT NULL DEFAULT 'YMIR',
  `szPointOn2` enum('NONE','MAX_HP','MAX_SP','HP_REGEN','SP_REGEN','BLOCK','HP','SP','ATT_GRADE','DEF_GRADE','MAGIC_ATT_GRADE','MAGIC_DEF_GRADE','BOW_DISTANCE','MOV_SPEED','ATT_SPEED','POISON_PCT','RESIST_RANGE','RESIST_MELEE','CASTING_SPEED','REFLECT_MELEE','ATT_BONUS','DEF_BONUS','RESIST_NORMAL','DODGE','KILL_HP_RECOVER','KILL_SP_RECOVER','HIT_HP_RECOVER','HIT_SP_RECOVER','CRITICAL','MANASHIELD','SKILL_DAMAGE_BONUS','NORMAL_HIT_DAMAGE_BONUS') NOT NULL DEFAULT 'NONE',
  `szPointPoly2` varchar(100) NOT NULL DEFAULT '',
  `szDurationPoly2` varchar(100) NOT NULL DEFAULT '',
  `setAffectFlag2` enum('YMIR','INVISIBILITY','SPAWN','POISON','SLOW','STUN','DUNGEON_READY','DUNGEON_UNIQUE','BUILDING_CONSTRUCTION_SMALL','BUILDING_CONSTRUCTION_LARGE','BUILDING_UPGRADE','MOV_SPEED_POTION','ATT_SPEED_POTION','FISH_MIND','JEONGWIHON','GEOMGYEONG','CHEONGEUN','GYEONGGONG','EUNHYUNG','GWIGUM','TERROR','JUMAGAP','HOSIN','BOHO','KWAESOK','MANASHIELD','MUYEONG','REVIVE_INVISIBLE','FIRE','GICHEON','JEUNGRYEOK','TANHWAN_DASH','PABEOP','CHEONGEUN_WITH_FALL','POLYMORPH','WAR_FLAG1','WAR_FLAG2','WAR_FLAG3','CHINA_FIREWORK','HAIR','GERMANY','RAMADAN_RING','BLEEDING','RED_POSSESSION','BLUE_POSSESSION') NOT NULL DEFAULT 'YMIR',
  `szPointOn3` varchar(100) NOT NULL DEFAULT 'NONE',
  `szPointPoly3` varchar(100) NOT NULL DEFAULT '',
  `szDurationPoly3` varchar(100) NOT NULL DEFAULT '',
  `szGrandMasterAddSPCostPoly` varchar(100) NOT NULL DEFAULT '',
  `prerequisiteSkillVnum` int(11) NOT NULL DEFAULT '0',
  `prerequisiteSkillLevel` int(11) NOT NULL DEFAULT '0',
  `eSkillType` enum('NORMAL','MELEE','RANGE','MAGIC') NOT NULL DEFAULT 'NORMAL',
  `iMaxHit` tinyint(4) NOT NULL DEFAULT '0',
  `szSplashAroundDamageAdjustPoly` varchar(100) NOT NULL DEFAULT '1',
  `dwTargetRange` int(11) NOT NULL DEFAULT '1000',
  `dwSplashRange` int(10) unsigned NOT NULL DEFAULT '0',
  PRIMARY KEY (`dwVnum`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `sms_pool`
--

DROP TABLE IF EXISTS `sms_pool`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `sms_pool` (
  `id` int(11) NOT NULL AUTO_INCREMENT,
  `server` int(11) NOT NULL DEFAULT '0',
  `sender` varchar(32) DEFAULT NULL,
  `receiver` varchar(100) NOT NULL DEFAULT '',
  `mobile` varchar(32) DEFAULT NULL,
  `sent` enum('N','Y') NOT NULL DEFAULT 'N',
  `msg` varchar(80) DEFAULT NULL,
  PRIMARY KEY (`id`) USING BTREE,
  KEY `sent_idx` (`sent`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `string`
--

DROP TABLE IF EXISTS `string`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `string` (
  `name` varchar(64) NOT NULL DEFAULT '',
  `text` text,
  PRIMARY KEY (`name`) USING BTREE
) ENGINE=MyISAM DEFAULT CHARSET=latin1 ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Table structure for table `worldboss`
--

DROP TABLE IF EXISTS `worldboss`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!40101 SET character_set_client = utf8 */;
CREATE TABLE `worldboss` (
  `pid` int(11) NOT NULL DEFAULT '0' COMMENT 'Player ID',
  `record` varchar(255) COLLATE utf8mb4_unicode_ci DEFAULT '0' COMMENT 'Record',
  `start_time` datetime DEFAULT NULL COMMENT 'Start Time',
  `empire` varchar(255) COLLATE utf8mb4_unicode_ci DEFAULT '0' COMMENT 'Empire',
  `guild_id` int(11) DEFAULT '0' COMMENT 'Guild ID',
  PRIMARY KEY (`pid`) USING BTREE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci ROW_FORMAT=DYNAMIC;
/*!40101 SET character_set_client = @saved_cs_client */;

--
-- Dumping events for database 'player'
--

--
-- Dumping routines for database 'player'
--
/*!40103 SET TIME_ZONE=@OLD_TIME_ZONE */;

/*!40101 SET SQL_MODE=@OLD_SQL_MODE */;
/*!40014 SET FOREIGN_KEY_CHECKS=@OLD_FOREIGN_KEY_CHECKS */;
/*!40014 SET UNIQUE_CHECKS=@OLD_UNIQUE_CHECKS */;
/*!40101 SET CHARACTER_SET_CLIENT=@OLD_CHARACTER_SET_CLIENT */;
/*!40101 SET CHARACTER_SET_RESULTS=@OLD_CHARACTER_SET_RESULTS */;
/*!40101 SET COLLATION_CONNECTION=@OLD_COLLATION_CONNECTION */;
/*!40111 SET SQL_NOTES=@OLD_SQL_NOTES */;

-- Dump completed
