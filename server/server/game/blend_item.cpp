
#include "stdafx.h"
#include "constants.h"
#include "log.h"
#include "locale_service.h"
#include "item.h"
#include "blend_item.h"
#include "char.h"
#include "config.h"

#include <fstream>
#include <rapidjson/document.h>
#include <rapidjson/istreamwrapper.h>
#include <rapidjson/error/en.h>

static int32_t FN_random_index()
{
	int	percent = number(1, 100);

	if (percent <= 10) // level 1 :10%
		return 0;
	if (percent <= 30) // level 2 : 20%
		return 1;
	if (percent <= 70) // level 3 : 40%
		return 2;
	if (percent <= 90) // level 4 : 20%
		return 3;
	return 4; // level 5 : 10%
}

static int32_t FN_ECS_random_index()
{
	int	percent = number(1, 100);

	if (percent <= 5) // level 1 : 5%
		return 0;
	if (percent <= 15) // level 2 : 10%
		return 1;
	if (percent <= 60) // level 3 : 45%
		return 2;
	if (percent <= 85) // level 4 : 25%
		return 3;
	return 4; // level 5 : 15%
}

CBlendItem::~CBlendItem()
{
	blend_.clear();
}

bool CBlendItem::Initialize()
{
	if (g_bAuthServer)
		return true;

	if (!blend_.empty())
		blend_.clear();

	char filename[PATH_MAX] = {};
	snprintf(filename, sizeof(filename), "%s/blend.json", LocaleService_GetBasePath().c_str());
	std::ifstream ifs(filename, std::ios::in);

	if (ifs.fail())
		return false;

	rapidjson::Document document;
	rapidjson::IStreamWrapper isw(ifs);
	document.ParseStream(isw);

	if (document.HasParseError())
	{
		sys_err("%s parse failed! Error: %s offset: %u", filename, GetParseError_En(document.GetParseError()), document.GetErrorOffset());
		return false;
	}

	if (!document.IsArray())
	{
		sys_err("%s: the document isn't an array", filename);
		return false;
	}

	/* ex)
		{
			"item_vnum": 50821,
			"apply_type": "CRITICAL_PCT",
			"apply_value": [20, 20, 20, 20, 20],
			"apply_duration": [600, 600, 600, 600, 600]
		},
	*/

	for (size_t i = 0; i < document.Size(); ++i)
	{
		const auto& v = document[i];
		if (!v.IsObject())
		{
			sys_err("%s: the document(%u) isn't an object", filename, i);
			return false;
		}

		auto has_member = [&] {
			for (const auto& j : { "item_vnum", "apply_type", "apply_value", "apply_duration" })
			{
				// if (!v.HasMember(j))
				// {
					// sys_err("%s: the document(%u) has no member(%s)", filename, i, j);
					// return false;
				// }
			};
			
			return true;
		};

		if (!has_member())
			return false;

		const auto& item_vnum = v["item_vnum"];
		if (!item_vnum.IsUint())
		{
			sys_err("%s: document(%u) the `item_vnum` isn't an integer", filename, i);
			return false;
		}

		if (blend_.find(item_vnum.GetUint()) != blend_.end())
		{
			sys_err("%s: vnum(%u) already exist", filename, item_vnum.GetUint());
			return false;
		}

		const auto& apply_type = v["apply_type"];
		if (!apply_type.IsString())
		{
			sys_err("%s: document(%u) the `apply_type` isn't a string", filename, i);
			return false;
		}

		const auto get_apply_type = FN_get_apply_type(apply_type.GetString());
		if (!get_apply_type)
		{
			sys_err("%s: unknown apply_type(%s)", filename, apply_type.GetString());
			return false;
		}

		const auto& apply_value = v["apply_value"];
		if (!apply_value.IsArray())
		{
			sys_err("%s: document(%u) the `apply_value` isn't an array", filename, i);
			return false;
		}

		if (blend::item_max_value != apply_value.Size())
		{
			sys_err("%s: document(%u) `apply_value` wrong array size [%d != %u]", filename, i, blend::item_max_value, apply_value.Size());
			return false;
		}

		const auto& apply_duration = v["apply_duration"];
		if (!apply_duration.IsArray())
		{
			sys_err("%s: document(%u) the `apply_duration` isn't an array", filename, i);
			return false;
		}

		if (blend::item_max_value != apply_duration.Size())
		{
			sys_err("%s: document(%u) `apply_duration` wrong array size [%d != %u]", filename, i, blend::item_max_value, apply_value.Size());
			return false;
		}

		auto blend_item = std::make_shared<blend>();
		blend_item->apply_type = get_apply_type;

		for (auto n = 0; n < blend::item_max_value; ++n)
		{
			blend_item->apply_value[n] = apply_value[n].GetUint();
			blend_item->apply_duration[n] = apply_duration[n].GetUint();
		}
		blend_.emplace(item_vnum.GetUint(), blend_item);
	}
	return true;
}

bool CBlendItem::SetItemValue(LPITEM pkItem)
{
	if (!pkItem)
		return false;

	const auto iterator = blend_.find(pkItem->GetVnum());
	if (iterator == blend_.end())
		return false;

	const auto ecs_random = pkItem->GetVnum() == 51002;
	const auto apply_type = iterator->second->apply_type;
	const auto get_random_index = ecs_random ? FN_ECS_random_index() : FN_random_index();

	assert(blend::item_max_value > get_random_index);

	const auto apply_value = iterator->second->apply_value[get_random_index];
	const auto apply_duration = iterator->second->apply_duration[get_random_index];

	pkItem->SetSocket(0, apply_type);
	pkItem->SetSocket(1, apply_value);
	if (apply_duration <= 0)
		pkItem->SetSocket(2, INFINITE_AFFECT_DURATION);
	else
		pkItem->SetSocket(2, apply_duration);
	pkItem->SetSocket(3, false);
	sys_log(1, "blend_item : type : %d, value : %d, du : %d", apply_type, apply_value, apply_duration);
	return true;
}
