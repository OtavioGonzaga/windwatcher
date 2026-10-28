-- All index members have score zero; time is compared as fixed-width hex strings.
local p = KEYS[1]
local function key(id) return p .. ':job:' .. id end
local function index(s) return p .. ':index:status:' .. s end
local function due(s) return p .. ':index:due:' .. s end
local all = p .. ':index:all'
local completed = p .. ':index:completed-at'
local function corrupt(reason) error('WW_CORRUPT:' .. reason, 0) end
local function check_type(k, wanted)
    local t = redis.call('TYPE', k).ok
    if t ~= 'none' and t ~= wanted then corrupt('key-type') end
end
for _, k in ipairs({all, completed, index('pending'),index('leased'),index('completed'),index('failed'),due('pending'),due('leased')}) do
    check_type(k, 'zset')
end
local function hash(id)
    local k = key(id)
    check_type(k, 'hash')
    if redis.call('EXISTS', k) == 0 then corrupt('missing-indexed-job') end
    return k
end
local function stored_time(k, field)
    local t = redis.call('HGET',k,field)
    if not t or #t ~= 16 or string.find(t,'[^0-9a-f]') then corrupt('invalid-timestamp') end
    return t
end
local function member(k, id) return stored_time(k,'created_at') .. ':' .. id end
local function counters(k)
    local a = tonumber(redis.call('HGET',k,'attempts'))
    local m = tonumber(redis.call('HGET',k,'max_attempts'))
    if not a or not m or a < 0 or m < 1 or a > m or m > 4294967295 or a ~= math.floor(a) or m ~= math.floor(m) then corrupt('invalid-attempts') end
    return a,m
end
local function clear_lease(k)
    redis.call('HDEL',k,'lease_id','lease_owner','lease_expires_at')
end
local function remove_due(k, id, s)
    local field = s == 'pending' and 'available_at' or 'lease_expires_at'
    redis.call('ZREM',due(s),stored_time(k,field) .. ':' .. id)
end
local function move(k, id, old, new)
    local m = member(k,id)
    redis.call('ZREM',index(old),m)
    redis.call('ZADD',index(new),0,m)
    redis.call('HSET',k,'status',new)
end
local function exhausted(k,id,now)
    member(k,id)
    remove_due(k,id,'leased')
    move(k,id,'leased','failed')
    clear_lease(k)
    redis.call('HSET',k,'failed_at',now,'last_error_code','queue.attempts_exhausted','last_error_message','maximum attempts exhausted')
end
local function fenced(k,lease,now)
    return redis.call('HGET',k,'status') == 'leased'
        and redis.call('HGET',k,'lease_id') == lease
        and stored_time(k,'lease_expires_at') > now
end
local function first_due(s,now)
    return redis.call('ZRANGEBYLEX',due(s),'-','[' .. now .. ':~','LIMIT',0,1)[1]
end
local function job_id(m) return string.sub(m,18) end
